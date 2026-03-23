//! Context switch — save/restore callee-saved state between tasks.
//!
//! [`context_switch`] is a thin Rust wrapper that saves the current task's
//! callee-saved registers into `*old` and loads them from `*new`.  Only
//! callee-saved registers are handled here because the Rust calling convention
//! guarantees the compiler has already saved caller-saved registers at the
//! call site.
//!
//! `mstatus` is saved and restored in full so that the `MPP` field survives
//! the switch.  When user-mode tasks are introduced (Sprint 3 stretch), `MPP`
//! is what makes `mret` drop back to User mode rather than staying in Machine
//! mode.

use super::TrapFrame;
use core::arch::global_asm;

// The assembly stub saves/loads only the callee-saved set:
//   ra, sp, s0–s11 (14 registers × 4 bytes = 56 bytes in TrapFrame.regs)
// plus mepc and mstatus from the CSR fields.
//
// Caller-saved registers (a0–a7, t0–t6) are not touched because the compiler
// has already spilled them to the caller's stack frame before the call.
global_asm!(
    ".section .text",
    ".global __context_switch",
    ".align 2",
    "__context_switch:",
    // a0 = *mut TrapFrame (old), a1 = *const TrapFrame (new)

    // --- Save callee-saved GPRs into old TrapFrame -------------------------
    // ra  = regs[1]  (offset 4)
    "sw  ra,   4(a0)",
    // sp  = regs[2]  (offset 8)
    "sw  sp,   8(a0)",
    // s0  = regs[8]  (offset 32)
    "sw  s0,  32(a0)",
    // s1  = regs[9]  (offset 36)
    "sw  s1,  36(a0)",
    // s2–s11 = regs[18–27] (offsets 72–108)
    "sw  s2,  72(a0)",
    "sw  s3,  76(a0)",
    "sw  s4,  80(a0)",
    "sw  s5,  84(a0)",
    "sw  s6,  88(a0)",
    "sw  s7,  92(a0)",
    "sw  s8,  96(a0)",
    "sw  s9, 100(a0)",
    "sw s10, 104(a0)",
    "sw s11, 108(a0)",

    // Save mepc (return address after context restore) and mstatus (MPP).
    "csrr t0, mepc",    "sw t0, 128(a0)",
    "csrr t0, mstatus", "sw t0, 132(a0)",

    // --- Load callee-saved GPRs from new TrapFrame -------------------------
    "lw  ra,   4(a1)",
    "lw  sp,   8(a1)",
    "lw  s0,  32(a1)",
    "lw  s1,  36(a1)",
    "lw  s2,  72(a1)",
    "lw  s3,  76(a1)",
    "lw  s4,  80(a1)",
    "lw  s5,  84(a1)",
    "lw  s6,  88(a1)",
    "lw  s7,  92(a1)",
    "lw  s8,  96(a1)",
    "lw  s9, 100(a1)",
    "lw s10, 104(a1)",
    "lw s11, 108(a1)",

    "lw t0, 128(a1)", "csrw mepc,    t0",
    "lw t0, 132(a1)", "csrw mstatus, t0",

    "ret",
);

extern "C" {
    fn __context_switch(old: *mut TrapFrame, new: *const TrapFrame);
}

/// Save the current execution context into `old` and resume from `new`.
///
/// Only callee-saved registers and the two resumption CSRs (`mepc`,
/// `mstatus`) are transferred; caller-saved registers are not touched.
///
/// # Safety
///
/// Both `old` and `new` must point to valid, properly aligned `TrapFrame`
/// values.  `new.mepc` must be a valid instruction address to resume from.
/// Calling this before the trap-entry stub has initialised a frame (i.e.
/// before Sprint 4's scheduler is wired up) will produce undefined behaviour.
#[inline]
pub unsafe fn context_switch(old: *mut TrapFrame, new: *const TrapFrame) {
    // SAFETY: delegated to caller; see function-level Safety section.
    unsafe { __context_switch(old, new) }
}
