//! Machine-mode trap entry/exit and the Rust-level trap dispatcher.
//!
//! The entry stub (`__trap_entry`) is written in assembly and placed in the
//! `.trap` section so the linker script can locate it for `mtvec`.  It saves
//! the full machine state into a `TrapFrame` on the kernel stack, then calls
//! the Rust [`trap_handler`] function.  On return it restores the state and
//! executes `mret`.
//!
//! # Why `global_asm!` instead of `#[naked]`?
//!
//! `#[naked]` functions are callable from Rust and subject to the ABI;
//! `global_asm!` gives a raw assembly block with no Rust function frame
//! whatsoever, which is what we need for a trap entry that runs before any
//! register has been touched.

use core::arch::global_asm;

use super::TrapFrame;
use crate::uart;

// Field byte offsets into TrapFrame (must match #[repr(C)] layout).
// regs: 32 × 4 = 128 bytes (offsets 0x00–0x7C)
// mepc:    0x80
// mstatus: 0x84
// mcause:  0x88
// mtval:   0x8C
global_asm!(
    // Place this code in the .trap section so the linker script's
    // KEEP(*(.trap)) directive keeps it and the linker knows to use
    // it for mtvec in direct-mode (lowest 2 bits = 0).
    ".section .trap, \"ax\"",
    ".global __trap_entry",
    ".align 2",                     // mtvec requires 4-byte alignment
    "__trap_entry:",

    // --- Allocate TrapFrame on the stack (36 × 4 = 144 bytes) ---
    "addi sp, sp, -144",

    // Save all 32 GPRs.  x0 (zero) is written as 0 for index uniformity.
    "sw   x0,   0(sp)",
    "sw   ra,   4(sp)",
    "sw   sp,   8(sp)",             // sp already decremented; records the trap-time value
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

    // Save trap CSRs using t0 as a scratch register.
    // t0 was already saved at offset 20, so reusing it is safe.
    "csrr t0, mepc",    "sw t0, 128(sp)",   // mepc    → offset 0x80
    "csrr t0, mstatus", "sw t0, 132(sp)",   // mstatus → offset 0x84
    "csrr t0, mcause",  "sw t0, 136(sp)",   // mcause  → offset 0x88
    "csrr t0, mtval",   "sw t0, 140(sp)",   // mtval   → offset 0x8C

    // Pass TrapFrame pointer as the first (and only) argument.
    "mv a0, sp",
    "call trap_handler",

    // Restore CSRs from (possibly modified) TrapFrame before GPRs so that
    // t0 is still available as a scratch register.
    "lw t0, 128(sp)", "csrw mepc,    t0",
    "lw t0, 132(sp)", "csrw mstatus, t0",
    // mcause and mtval are read-only from software; no restore needed.

    // Restore all GPRs.
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

    // sp last: restoring it before the other registers would corrupt the frame.
    "lw   sp,   8(sp)",

    "mret",
);

/// Machine-mode trap dispatcher.
///
/// Called from `__trap_entry` with a pointer to the fully saved [`TrapFrame`].
/// Modifying `frame.mepc` before returning will change the resume address.
///
/// # Safety
///
/// Must only be called from `__trap_entry` with a valid, stack-allocated
/// `TrapFrame`.  `frame` must remain valid for the duration of this function.
#[no_mangle]
pub unsafe extern "C" fn trap_handler(frame: *mut TrapFrame) {
    // SAFETY: __trap_entry guarantees the frame is fully initialised and
    // exclusively owned by this call (no re-entrant traps in M-mode).
    let frame = unsafe { &mut *frame };

    // Bit 31 set → interrupt; clear → synchronous exception.
    let is_interrupt = (frame.mcause >> 31) & 1 == 1;
    let cause_code   = frame.mcause & !(1 << 31);

    if is_interrupt {
        match cause_code {
            // Machine timer interrupt (MTIE, cause code 7).
            7 => crate::clint::timer_isr(),
            _ => {
                uart::uart_puts("[TRAP] unhandled interrupt cause=");
                uart::uart_print_hex(frame.mcause);
                uart::uart_puts("\n");
            }
        }
    } else {
        uart::uart_puts("[TRAP] exception mcause=");
        uart::uart_print_hex(frame.mcause);
        uart::uart_puts(" mepc=");
        uart::uart_print_hex(frame.mepc);
        uart::uart_puts(" mtval=");
        uart::uart_print_hex(frame.mtval);
        uart::uart_puts("\n");
        // Advance past the faulting instruction so we can continue.
        // For a real kernel this would terminate the offending task; for now
        // we skip forward by the compressed (2-byte) or full (4-byte) width.
        // bit 0 and bit 1 both set → 32-bit instruction.
        let instr_len = if (frame.mepc & 0x3) == 0x3 { 4 } else { 2 };
        frame.mepc += instr_len;
    }
}
