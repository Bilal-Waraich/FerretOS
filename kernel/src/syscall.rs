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
//! | `a0` | return value ([`ENOSYS`] for an unknown number) |
//!
//! All other registers are preserved: the trap path saves and restores the
//! full register file.

use crate::context::{TrapFrame, REG_A0, REG_A1, REG_A7};
use crate::{clint, scheduler, uart};

/// Give up the CPU; the caller re-enters the ready queue behind its peers.
pub const SYS_YIELD: usize = 0;
/// Block for `a0` timer ticks (minimum 1).
pub const SYS_SLEEP: usize = 1;
/// Write `a1` bytes starting at address `a0` to the console as one unit.
pub const SYS_LOG: usize = 2;
/// Return the current tick count.
pub const SYS_NOW: usize = 3;

/// Return value for an unknown syscall number.
pub const ENOSYS: usize = usize::MAX;

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
            let len = a1.min(LOG_MAX_LEN);
            // SAFETY: tasks currently run in M-mode and are trusted to pass a
            // valid (address, length) pair; bounding against the caller's
            // memory region comes with U-mode isolation (#86).  Interrupts
            // are disabled, so the message is written without interleaving.
            let bytes = unsafe { core::slice::from_raw_parts(a0 as *const u8, len) };
            for &b in bytes {
                // SAFETY: single hart, interrupts disabled — no concurrent
                // UART writer exists.
                unsafe { uart::uart_putchar(b) };
            }
            f.regs[REG_A0] = len;
            frame
        }
        SYS_NOW => {
            f.regs[REG_A0] = clint::ticks() as usize;
            frame
        }
        _ => {
            f.regs[REG_A0] = ENOSYS;
            frame
        }
    }
}
