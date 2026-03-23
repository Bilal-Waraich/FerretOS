//! context — RISC-V machine-mode execution context.
//!
//! This module owns the `TrapFrame` type, which is the contract between the
//! assembly trap entry stub and the Rust trap handler.  Every time the
//! processor takes an exception or interrupt, the entry stub saves the current
//! register state into a `TrapFrame` on the kernel stack and passes a pointer
//! to it as the first argument of `trap_handler`.

/// The complete register state captured on every trap (exception or interrupt).
///
/// The assembly entry stub saves all 32 general-purpose registers and the four
/// trap-relevant CSRs into this struct before calling `trap_handler`.  On
/// return the same stub restores the struct back into the machine registers
/// and executes `mret`, resuming whatever code was interrupted.
///
/// # Layout
///
/// `#[repr(C)]` is mandatory: the assembly stub accesses fields by hardcoded
/// byte offsets, so the Rust compiler must not reorder them.
///
/// # Register conventions (RISC-V integer ABI)
///
/// | Index | ABI name | Role                        | Saved by |
/// |-------|----------|-----------------------------|----------|
/// | 0     | zero     | Hard-wired zero             | —        |
/// | 1     | ra       | Return address              | Caller   |
/// | 2     | sp       | Stack pointer               | Callee   |
/// | 3     | gp       | Global pointer              | —        |
/// | 4     | tp       | Thread pointer              | —        |
/// | 5–7   | t0–t2    | Temporaries                 | Caller   |
/// | 8–9   | s0–s1    | Saved registers             | Callee   |
/// | 10–17 | a0–a7    | Function arguments / return | Caller   |
/// | 18–27 | s2–s11   | Saved registers             | Callee   |
/// | 28–31 | t3–t6    | Temporaries                 | Caller   |
///
/// We save *all* registers regardless of convention so that the interrupted
/// context can be resumed transparently — the trap handler does not know what
/// calling convention the interrupted code was using.
#[repr(C)]
pub struct TrapFrame {
    /// General-purpose registers x0–x31 in architectural order.
    ///
    /// x0 (zero) is always zero; it is included for index uniformity so that
    /// `regs[n]` always corresponds to register xN without a special case.
    pub regs: [usize; 32],

    /// Machine Exception Program Counter — the address the hart will return to
    /// on `mret`.  For synchronous exceptions this is the faulting instruction;
    /// for interrupts it is the next instruction that would have executed.
    pub mepc: usize,

    /// Machine Status Register snapshot.
    ///
    /// Preserving `mstatus` is essential for correct `mret` behaviour: the
    /// `MPP` field (bits 12:11) encodes the privilege level to return to.
    /// When PMP and user-mode tasks are introduced (Sprint 3 stretch), this
    /// field is what lets `mret` drop back from Machine to User mode.
    pub mstatus: usize,

    /// Machine Cause Register — encodes why the trap was taken.
    ///
    /// Bit 31 (XLEN-1) set → interrupt; clear → exception.
    /// Bits 30:0 → interrupt/exception code (e.g. 7 = machine timer interrupt).
    pub mcause: usize,

    /// Machine Trap Value — auxiliary information about the trap.
    ///
    /// For instruction/load/store address-misaligned and access-fault
    /// exceptions this holds the faulting address.  Zero otherwise.
    pub mtval: usize,
}
