//! Kernel side of the syscall interface.
//!
//! Tasks request kernel services with `ecall` (see [`crate::sys`] for the
//! task-side wrappers).  The trap handler routes environment-call exceptions
//! here with the caller's saved [`TrapFrame`], `mepc` already advanced past
//! the `ecall`.
//!
//! # ABI
//!
//! | Register | Role |
//! |---|---|
//! | `a7` | syscall number |
//! | `a0`, `a1` | arguments |
//! | `a0` | return value, or an error code (`usize::MAX - k`, see below) |
//!
//! All other registers are preserved: the trap path saves and restores the
//! full register file.
//!
//! # Argument validation
//!
//! Tasks run in U-mode and are untrusted.  The kernel runs in M-mode, where
//! PMP does not apply, so it must never dereference a task-supplied pointer
//! without first checking that the whole buffer lies in memory the task may
//! read itself: its own memory region and stack, or the shared read-only
//! flash image (code and string constants).  See [`buffer_readable`].

use crate::capability::CONSOLE_CAP_BIT;
use crate::context::{TrapFrame, REG_A0, REG_A1, REG_A2, REG_A3, REG_A7};
use crate::ipc::kernel::{channel, Message, MSG_MAX};
use crate::memory::task::{BlockReason, TaskDescriptor};
use crate::{clint, scheduler, uart};

/// Give up the CPU; the caller re-enters the ready queue behind its peers.
pub const SYS_YIELD: usize = 0;
/// Block for `a0` timer ticks (minimum 1).
pub const SYS_SLEEP: usize = 1;
/// Write `a1` bytes starting at address `a0` to the console as one unit.
pub const SYS_LOG: usize = 2;
/// Return the current tick count.
pub const SYS_NOW: usize = 3;
/// Send `a2` bytes at `a1` on channel `a0`; blocks while the channel is full
/// unless `a3 & FLAG_NONBLOCK`.
pub const SYS_SEND: usize = 4;
/// Receive one message from channel `a0` into the `a2`-byte buffer at `a1`;
/// blocks while the channel is empty unless `a3 & FLAG_NONBLOCK`.  Returns
/// the message length (the copy is truncated to the buffer size).
pub const SYS_RECEIVE: usize = 5;

/// `a3` flag for `send` / `receive`: return [`EAGAIN`] instead of blocking.
pub const FLAG_NONBLOCK: usize = 1;

/// Error: unknown syscall number.
pub const ENOSYS: usize = usize::MAX;
/// Error: the caller lacks the capability this syscall requires.
pub const EPERM: usize = usize::MAX - 1;
/// Error: a buffer argument is not accessible by the caller.
pub const EFAULT: usize = usize::MAX - 2;
/// Error: the operation would block and `FLAG_NONBLOCK` was given.
pub const EAGAIN: usize = usize::MAX - 3;
/// Error: an argument is out of range (e.g. message longer than `MSG_MAX`).
pub const EINVAL: usize = usize::MAX - 4;

// Error codes must be distinct and never collide with a valid byte count.
const _: () = assert!(ENOSYS > EPERM && EPERM > EFAULT && EFAULT > EAGAIN && EAGAIN > EINVAL);
const _: () = assert!(EINVAL > LOG_MAX_LEN);

/// Whether `[ptr, ptr + len)` lies entirely inside one of `ranges`
/// (each a half-open `[start, end)` interval).
///
/// Overflow-safe: a buffer whose end would wrap past `usize::MAX` is
/// rejected.  An empty buffer is accepted (nothing is read).
pub fn buffer_readable(ptr: usize, len: usize, ranges: &[(usize, usize)]) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = ptr.checked_add(len) else { return false };
    ranges.iter().any(|&(start, stop)| start <= ptr && end <= stop)
}

/// Whether `task` holds the console capability required by [`SYS_LOG`].
pub fn may_log(task: &TaskDescriptor) -> bool {
    task.shared_cap_mask & CONSOLE_CAP_BIT != 0
}

/// Address range of the read-only flash image (`.text` + `.rodata`), which
/// every task may read.
fn flash_image() -> (usize, usize) {
    #[cfg(target_arch = "riscv32")]
    {
        extern "C" {
            static _stext: u8;
            /// Load address of `.data`, which riscv-rt places immediately
            /// after `.rodata` in flash.
            static _sidata: u8;
        }
        // Only the addresses of the linker symbols are taken, never their
        // contents, so no `unsafe` is needed.
        (core::ptr::addr_of!(_stext) as usize, core::ptr::addr_of!(_sidata) as usize)
    }
    #[cfg(not(target_arch = "riscv32"))]
    (0, 0)
}

/// Longest message accepted by [`SYS_LOG`]; longer messages are truncated.
pub const LOG_MAX_LEN: usize = 256;

/// Execute the syscall described by `frame` and return the frame to resume.
///
/// The returned frame is the caller's own unless the syscall blocked or
/// yielded, in which case it is the next task chosen by the scheduler.
///
/// # Safety
///
/// `frame` must be the saved frame of the currently running task, passed by
/// the trap handler with interrupts disabled.
pub unsafe fn dispatch(frame: *mut TrapFrame) -> *mut TrapFrame {
    // SAFETY: the trap handler owns `frame` exclusively for this call.
    let f = unsafe { &mut *frame };
    let a0 = f.regs[REG_A0];
    let a1 = f.regs[REG_A1];

    match f.regs[REG_A7] {
        SYS_YIELD => {
            f.regs[REG_A0] = 0;
            scheduler::yield_current(frame)
        }
        SYS_SLEEP => {
            f.regs[REG_A0] = 0;
            scheduler::sleep_current(frame, a0 as u32)
        }
        SYS_LOG => {
            f.regs[REG_A0] = sys_log(a0, a1.min(LOG_MAX_LEN));
            frame
        }
        SYS_NOW => {
            f.regs[REG_A0] = clint::ticks() as usize;
            frame
        }
        SYS_SEND => sys_send(frame, a0, a1, f.regs[REG_A2], f.regs[REG_A3]),
        SYS_RECEIVE => sys_receive(frame, a0, a1, f.regs[REG_A2], f.regs[REG_A3]),
        _ => {
            f.regs[REG_A0] = ENOSYS;
            frame
        }
    }
}

/// Memory `task` may read: its region, its stack, and the flash image.
fn readable_ranges(task: &TaskDescriptor) -> [(usize, usize); 3] {
    [
        (task.memory_start, task.memory_end),
        (task.stack_base, task.stack_base + task.stack_size),
        flash_image(),
    ]
}

/// Memory `task` may write: its region and its stack (never flash).
fn writable_ranges(task: &TaskDescriptor) -> [(usize, usize); 2] {
    [(task.memory_start, task.memory_end), (task.stack_base, task.stack_base + task.stack_size)]
}

/// Whether bit `ch` of `mask` is set, for any `ch` (out-of-range = false).
fn holds_endpoint(mask: u32, ch: usize) -> bool {
    ch < 32 && mask & (1 << ch) != 0
}

/// Arrange for the interrupted `ecall` to run again when the task resumes,
/// then block it.  Blocking syscalls are restartable: on wake-up the task
/// simply retries the operation, which now succeeds.
fn block_and_restart(frame: *mut TrapFrame, reason: BlockReason) -> *mut TrapFrame {
    // SAFETY: `frame` is the caller's saved frame (see `dispatch`).
    unsafe { (*frame).mepc -= 4 };
    scheduler::block_current(frame, reason)
}

/// Store `value` in the caller's `a0` and return `frame` unchanged.
fn ret(frame: *mut TrapFrame, value: usize) -> *mut TrapFrame {
    // SAFETY: `frame` is the caller's saved frame (see `dispatch`).
    unsafe { (*frame).regs[REG_A0] = value };
    frame
}

/// `SYS_SEND` body.
fn sys_send(frame: *mut TrapFrame, ch: usize, ptr: usize, len: usize, flags: usize) -> *mut TrapFrame {
    let Some(task) = scheduler::current_task() else { return ret(frame, EPERM) };
    let Some(chan) = channel(ch) else { return ret(frame, EINVAL) };
    if !holds_endpoint(task.sends_mask, ch) {
        return ret(frame, EPERM);
    }
    if len > MSG_MAX {
        return ret(frame, EINVAL);
    }
    if !buffer_readable(ptr, len, &readable_ranges(&task)) {
        return ret(frame, EFAULT);
    }
    if chan.is_full() {
        if flags & FLAG_NONBLOCK != 0 {
            return ret(frame, EAGAIN);
        }
        return block_and_restart(frame, BlockReason::Send(ch as u8));
    }
    // SAFETY: `[ptr, ptr + len)` lies in memory the caller may read (checked
    // above); interrupts are disabled, so it cannot change during the copy.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    let msg = Message::from_bytes(bytes).expect("len <= MSG_MAX checked above");
    // Cannot fail: not full, and this task is the channel's only producer.
    let _ = chan.try_send(msg);
    ret(frame, 0);
    scheduler::wake_blocked_on(BlockReason::Receive(ch as u8));
    scheduler::preempt_if_outranked(frame)
}

/// `SYS_RECEIVE` body.
fn sys_receive(frame: *mut TrapFrame, ch: usize, ptr: usize, cap: usize, flags: usize) -> *mut TrapFrame {
    let Some(task) = scheduler::current_task() else { return ret(frame, EPERM) };
    let Some(chan) = channel(ch) else { return ret(frame, EINVAL) };
    if !holds_endpoint(task.receives_mask, ch) {
        return ret(frame, EPERM);
    }
    if !buffer_readable(ptr, cap, &writable_ranges(&task)) {
        return ret(frame, EFAULT);
    }
    let Some(msg) = chan.try_receive() else {
        if flags & FLAG_NONBLOCK != 0 {
            return ret(frame, EAGAIN);
        }
        return block_and_restart(frame, BlockReason::Receive(ch as u8));
    };
    let n = msg.bytes().len().min(cap);
    // SAFETY: `[ptr, ptr + cap)` lies in the caller's own writable memory
    // (checked above) and `n <= cap`.
    unsafe { core::ptr::copy_nonoverlapping(msg.bytes().as_ptr(), ptr as *mut u8, n) };
    ret(frame, msg.bytes().len());
    scheduler::wake_blocked_on(BlockReason::Send(ch as u8));
    scheduler::preempt_if_outranked(frame)
}

/// `SYS_LOG` body: check capability and buffer, then write it to the UART.
///
/// Returns the number of bytes written, or [`EPERM`] / [`EFAULT`].
fn sys_log(ptr: usize, len: usize) -> usize {
    let Some(task) = scheduler::current_task() else { return EPERM };
    if !may_log(&task) {
        return EPERM;
    }
    // Task regions are linker-backed sections in TASKRAM, disjoint from
    // kernel RAM (#87), so accepting them cannot expose kernel data.
    if !buffer_readable(ptr, len, &readable_ranges(&task)) {
        return EFAULT;
    }
    // SAFETY: `[ptr, ptr + len)` was just checked to lie inside the caller's
    // own stack or the read-only flash image, both mapped and readable.
    // Interrupts are disabled, so the message is written without
    // interleaving and the buffer cannot change underneath us.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    for &b in bytes {
        // SAFETY: single hart, interrupts disabled — no concurrent UART writer.
        unsafe { uart::uart_putchar(b) };
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_inside_range_is_readable() {
        assert!(buffer_readable(0x1000, 16, &[(0x1000, 0x2000)]));
        assert!(buffer_readable(0x1ff0, 16, &[(0x1000, 0x2000)]));
    }

    #[test]
    fn buffer_crossing_range_end_is_rejected() {
        assert!(!buffer_readable(0x1ff8, 16, &[(0x1000, 0x2000)]));
        assert!(!buffer_readable(0x0ff8, 16, &[(0x1000, 0x2000)]));
    }

    #[test]
    fn buffer_spanning_two_adjacent_ranges_is_rejected() {
        // Each range must contain the whole buffer on its own.
        assert!(!buffer_readable(0x1ff8, 16, &[(0x1000, 0x2000), (0x2000, 0x3000)]));
    }

    #[test]
    fn wrapping_buffer_is_rejected() {
        assert!(!buffer_readable(usize::MAX - 4, 16, &[(0, usize::MAX)]));
    }

    #[test]
    fn empty_buffer_is_accepted() {
        assert!(buffer_readable(0xdead_0000, 0, &[]));
    }

    #[test]
    fn console_capability_gates_logging() {
        let mut t = TaskDescriptor::new(0, 1, 0, 16, 0, 0);
        assert!(!may_log(&t));
        t.shared_cap_mask = CONSOLE_CAP_BIT;
        assert!(may_log(&t));
        // Exclusive ownership of an unrelated peripheral grants nothing.
        t.shared_cap_mask = 0;
        t.exclusive_cap_mask = 1;
        assert!(!may_log(&t));
    }
}
