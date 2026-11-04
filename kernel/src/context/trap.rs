//! Machine-mode trap entry/exit and the Rust-level trap dispatcher.
//!
//! The entry stub (`__trap_entry`) is written in assembly and placed in the
//! `.trap` section so the linker script can locate it for `mtvec`.  It saves
//! the full machine state into a `TrapFrame` on the interrupted task's stack,
//! then calls the Rust [`trap_handler`] function, which returns the frame to
//! resume.  The stub switches `sp` to that frame, restores it, and executes
//! `mret` — so returning another task's frame *is* the context switch.
//!
//! # Why `global_asm!` instead of `#[naked]`?
//!
//! `#[naked]` functions are callable from Rust and subject to the ABI;
//! `global_asm!` gives a raw assembly block with no Rust function frame
//! whatsoever, which is what we need for a trap entry that runs before any
//! register has been touched.

#[cfg(target_arch = "riscv32")]
use core::arch::global_asm;

use super::TrapFrame;
use crate::uart;

// Field byte offsets into TrapFrame (must match #[repr(C)] layout).
// regs: 32 × 4 = 128 bytes (offsets 0x00–0x7C)
// mepc:    0x80
// mstatus: 0x84
// mcause:  0x88
// mtval:   0x8C
#[cfg(target_arch = "riscv32")]
global_asm!(
    // Place this code in the .trap section so the linker script's
    // KEEP(*(.trap)) directive keeps it and the linker knows to use
    // it for mtvec in direct-mode (lowest 2 bits = 0).
    ".section .trap, \"ax\"",
    ".global __trap_entry",
    ".align 2",                     // mtvec requires 4-byte alignment
    "__trap_entry:",

    // --- Allocate TrapFrame on the interrupted task's stack (144 bytes) ---
    "addi sp, sp, -144",

    // Save all GPRs except sp.  x0 (zero) is written as 0 for index uniformity.
    "sw   x0,   0(sp)",
    "sw   ra,   4(sp)",
    "sw   gp,  12(sp)",
    "sw   tp,  16(sp)",
    "sw   t0,  20(sp)",
    "sw   t1,  24(sp)",
    "sw   t2,  28(sp)",
    "sw   s0,  32(sp)",
    "sw   s1,  36(sp)",
    "sw   a0,  40(sp)",
    "sw   a1,  44(sp)",
    "sw   a2,  48(sp)",
    "sw   a3,  52(sp)",
    "sw   a4,  56(sp)",
    "sw   a5,  60(sp)",
    "sw   a6,  64(sp)",
    "sw   a7,  68(sp)",
    "sw   s2,  72(sp)",
    "sw   s3,  76(sp)",
    "sw   s4,  80(sp)",
    "sw   s5,  84(sp)",
    "sw   s6,  88(sp)",
    "sw   s7,  92(sp)",
    "sw   s8,  96(sp)",
    "sw   s9, 100(sp)",
    "sw  s10, 104(sp)",
    "sw  s11, 108(sp)",
    "sw   t3, 112(sp)",
    "sw   t4, 116(sp)",
    "sw   t5, 120(sp)",
    "sw   t6, 124(sp)",

    // regs[2] must hold the interrupted context's sp, i.e. the value before
    // the frame was allocated.  Storing the decremented sp here would make
    // every trap return 144 bytes lower on the stack.
    "addi t0, sp, 144",
    "sw   t0,   8(sp)",

    // Save trap CSRs using t0 as a scratch register (already saved above).
    "csrr t0, mepc",    "sw t0, 128(sp)",   // mepc    → offset 0x80
    "csrr t0, mstatus", "sw t0, 132(sp)",   // mstatus → offset 0x84
    "csrr t0, mcause",  "sw t0, 136(sp)",   // mcause  → offset 0x88
    "csrr t0, mtval",   "sw t0, 140(sp)",   // mtval   → offset 0x8C

    // trap_handler(frame) returns the frame to resume in a0 — the same frame,
    // or another task's saved frame when the scheduler switches.
    "mv   a0, sp",
    "call trap_handler",

    // --- Restore path; also the entry point for starting the first task ---
    ".global __restore_context",
    "__restore_context:",
    "mv   sp, a0",

    // Restore CSRs before GPRs so that t0 is still free as scratch.
    "lw t0, 128(sp)", "csrw mepc,    t0",
    "lw t0, 132(sp)", "csrw mstatus, t0",
    // mcause and mtval need no restore.

    "lw   ra,   4(sp)",
    "lw   gp,  12(sp)",
    "lw   tp,  16(sp)",
    "lw   t0,  20(sp)",
    "lw   t1,  24(sp)",
    "lw   t2,  28(sp)",
    "lw   s0,  32(sp)",
    "lw   s1,  36(sp)",
    "lw   a0,  40(sp)",
    "lw   a1,  44(sp)",
    "lw   a2,  48(sp)",
    "lw   a3,  52(sp)",
    "lw   a4,  56(sp)",
    "lw   a5,  60(sp)",
    "lw   a6,  64(sp)",
    "lw   a7,  68(sp)",
    "lw   s2,  72(sp)",
    "lw   s3,  76(sp)",
    "lw   s4,  80(sp)",
    "lw   s5,  84(sp)",
    "lw   s6,  88(sp)",
    "lw   s7,  92(sp)",
    "lw   s8,  96(sp)",
    "lw   s9, 100(sp)",
    "lw  s10, 104(sp)",
    "lw  s11, 108(sp)",
    "lw   t3, 112(sp)",
    "lw   t4, 116(sp)",
    "lw   t5, 120(sp)",
    "lw   t6, 124(sp)",

    // sp last: it is the base register for every load above.
    "lw   sp,   8(sp)",

    "mret",
);

#[cfg(target_arch = "riscv32")]
extern "C" {
    fn __restore_context(frame: *mut TrapFrame) -> !;
}

/// Start executing the task whose saved frame is `frame`.  Never returns.
///
/// Used once on the boot path to leave the boot stack and enter the first
/// task: it runs the trap exit sequence, so `mret` applies the frame's
/// `mstatus` (enabling interrupts via `MPIE`) and jumps to its `mepc`.
///
/// # Safety
///
/// `frame` must point to a valid frame built by [`super::init_frame`] (or
/// saved by the trap entry stub), and `mtvec` must already point at
/// `__trap_entry` so the first interrupt is handled.
#[cfg(target_arch = "riscv32")]
pub unsafe fn start_first_task(frame: *mut TrapFrame) -> ! {
    // SAFETY: delegated to the caller (see the function's Safety section).
    unsafe { __restore_context(frame) }
}

/// Machine-mode trap dispatcher.
///
/// Called from `__trap_entry` with a pointer to the fully saved [`TrapFrame`].
/// Returns the frame to resume: `frame` itself to continue the interrupted
/// task, or another task's saved frame to switch to it.  Modifying the
/// returned frame's `mepc` changes the resume address.
///
/// # Safety
///
/// Must only be called from `__trap_entry` with a valid, stack-allocated
/// `TrapFrame`.  `frame` must remain valid for the duration of this function.
#[no_mangle]
pub unsafe extern "C" fn trap_handler(frame: *mut TrapFrame) -> *mut TrapFrame {
    // SAFETY: __trap_entry passes a fully initialised frame that it owns
    // exclusively for the duration of this call (traps do not nest: MIE is
    // cleared on trap entry and stays clear until mret).
    let f = unsafe { &mut *frame };

    // Bit 31 set → interrupt; clear → synchronous exception.
    let is_interrupt = (f.mcause >> 31) & 1 == 1;
    let cause_code = f.mcause & !(1 << 31);

    if is_interrupt {
        match cause_code {
            // Machine timer interrupt (MTIE, cause code 7).
            7 => crate::scheduler::on_timer_interrupt(frame),
            _ => {
                uart::uart_puts("[TRAP] unhandled interrupt cause=");
                uart::uart_print_hex(f.mcause);
                uart::uart_puts("\n");
                frame
            }
        }
    } else {
        match cause_code {
            // Environment call from U-mode (8) or M-mode (11): a syscall.
            // `ecall` is always a 4-byte instruction; resume after it.
            8 | 11 => {
                f.mepc += 4;
                crate::syscall::dispatch(frame)
            }
            _ => fault_and_halt(f),
        }
    }
}

/// Print a fault record for an unexpected exception and halt.
///
/// Per CLAUDE.md §6.5 a runtime hardware fault is not recoverable: the
/// faulting task is never resumed.
fn fault_and_halt(f: &TrapFrame) -> ! {
    uart::uart_puts("[FAULT] exception mcause=");
    uart::uart_print_hex(f.mcause);
    uart::uart_puts(" mepc=");
    uart::uart_print_hex(f.mepc);
    uart::uart_puts(" mtval=");
    uart::uart_print_hex(f.mtval);
    uart::uart_puts(" task=");
    uart::uart_print_usize(crate::scheduler::current_task_idx() as usize);
    uart::uart_puts("\nSystem halted.\n");
    crate::halt()
}
