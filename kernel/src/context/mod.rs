//! Execution context — trap frame, initial task frames, and the trap path.
//!
//! `TrapFrame` is the contract between the assembly trap entry stub and the
//! Rust trap handler.  On every trap the stub saves all register state into a
//! `TrapFrame` on the interrupted task's stack and passes a pointer to
//! `trap_handler`, which returns the frame to resume.  A context switch is
//! therefore just returning a different task's saved frame: the stub loads
//! `sp` from the returned pointer and restores from there.

pub mod trap;

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
    /// `MPP` field (bits 12:11) encodes the privilege level to return to and
    /// `MPIE` (bit 7) becomes `MIE` after `mret`.
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

/// Size of a [`TrapFrame`] in bytes.  Must match the `addi sp, sp, -144` in
/// the trap entry stub, and stay a multiple of 16 (RISC-V stack alignment).
pub const TRAP_FRAME_SIZE: usize = core::mem::size_of::<TrapFrame>();
#[cfg(target_arch = "riscv32")]
const _: () = assert!(TRAP_FRAME_SIZE == 144);
const _: () = assert!(TRAP_FRAME_SIZE % 16 == 0);

/// Register index of `ra` (x1) in [`TrapFrame::regs`].
pub const REG_RA: usize = 1;
/// Register index of `sp` (x2).
pub const REG_SP: usize = 2;
/// Register index of `gp` (x3).
pub const REG_GP: usize = 3;
/// Register index of `a0` (x10) — first argument / return value.
pub const REG_A0: usize = 10;
/// Register index of `a1` (x11).
pub const REG_A1: usize = 11;
/// Register index of `a2` (x12).
pub const REG_A2: usize = 12;
/// Register index of `a3` (x13).
pub const REG_A3: usize = 13;
/// Register index of `a7` (x17) — syscall number.
pub const REG_A7: usize = 17;

/// `mstatus.MPP` = Machine mode (bits 12:11 = 0b11).
pub const MSTATUS_MPP_MACHINE: usize = 0b11 << 11;
/// `mstatus.MPP` = User mode (bits 12:11 = 0b00).
pub const MSTATUS_MPP_USER: usize = 0;
/// `mstatus.MPIE` (bit 7): interrupts are enabled after `mret`.
pub const MSTATUS_MPIE: usize = 1 << 7;

// ---------------------------------------------------------------------------
// Per-task kernel stacks
// ---------------------------------------------------------------------------

/// Size of each task's kernel stack in bytes.
///
/// Holds one `TrapFrame` plus the trap handler's call chain (scheduler,
/// syscall dispatch, UART diagnostics).  Traps never nest, so one frame per
/// task is the maximum.
pub const KERNEL_STACK_SIZE: usize = 1024;

/// A task's kernel stack: where its `TrapFrame` lives and where the trap
/// handler runs while servicing that task.
///
/// Kept in kernel memory, never in the task's own stack, so a U-mode task
/// cannot direct the kernel's frame writes by choosing its `sp` — M-mode
/// stores bypass PMP, so a frame saved at a task-controlled address could
/// overwrite arbitrary kernel memory.
#[repr(align(16))]
pub struct KernelStack(#[allow(dead_code)] [u8; KERNEL_STACK_SIZE]);

/// One kernel stack per registry slot, zero-initialised in `.bss`.
static mut KERNEL_STACKS: [KernelStack; crate::config::MAX_TASKS] =
    [const { KernelStack([0; KERNEL_STACK_SIZE]) }; crate::config::MAX_TASKS];

/// Kernel stack for traps taken on the boot path, before the first task runs.
static mut BOOT_TRAP_STACK: KernelStack = KernelStack([0; KERNEL_STACK_SIZE]);

/// One-past-the-end address of registry slot `slot`'s kernel stack.
///
/// # Panics
///
/// Panics if `slot >= MAX_TASKS`.
pub fn kernel_stack_top(slot: usize) -> usize {
    assert!(slot < crate::config::MAX_TASKS, "kernel_stack_top: slot out of range");
    // SAFETY: only the address of the element is formed; `slot` is in range.
    let base = unsafe { core::ptr::addr_of!(KERNEL_STACKS[slot]) } as usize;
    base + KERNEL_STACK_SIZE
}

/// One-past-the-end address of the boot-path trap stack.
pub fn boot_trap_stack_top() -> usize {
    core::ptr::addr_of!(BOOT_TRAP_STACK) as usize + KERNEL_STACK_SIZE
}

/// Build a task's first [`TrapFrame`] at the top of its kernel stack.
///
/// When the trap exit path restores this frame the task starts at `entry`
/// with `sp = user_stack_top`, the privilege mode and interrupt state given
/// by `mstatus`, and `ra` pointing at `exit_trap` so that a task body that
/// returns is caught rather than jumping to address 0.
///
/// The frame must sit exactly `TRAP_FRAME_SIZE` bytes below the top of the
/// kernel stack: the trap exit path derives the kernel stack top (for
/// `mscratch`) from the frame address.
///
/// Returns the frame address, which the scheduler stores as the task's
/// `stack_ptr`.
///
/// # Safety
///
/// `kernel_stack_top` must be the 16-byte-aligned one-past-the-end address of
/// a kernel stack of at least `TRAP_FRAME_SIZE` bytes that belongs to this
/// task and is not in use.
pub unsafe fn init_frame(
    kernel_stack_top: usize,
    user_stack_top: usize,
    entry: usize,
    exit_trap: usize,
    gp: usize,
    mstatus: usize,
) -> *mut TrapFrame {
    let frame = (kernel_stack_top - TRAP_FRAME_SIZE) as *mut TrapFrame;
    let mut regs = [0usize; 32];
    regs[REG_RA] = exit_trap;
    regs[REG_SP] = user_stack_top;
    regs[REG_GP] = gp;
    // SAFETY: the caller guarantees `[kernel_stack_top - TRAP_FRAME_SIZE,
    // kernel_stack_top)` lies inside an unused kernel stack owned by this
    // task; the address is 16-byte aligned, which satisfies `TrapFrame`'s
    // alignment.
    unsafe {
        frame.write(TrapFrame { regs, mepc: entry, mstatus, mcause: 0, mtval: 0 });
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_frame_places_frame_below_kernel_stack_top() {
        #[repr(align(16))]
        struct Buf([u8; 512]);
        let mut buf = Buf([0; 512]);
        let top = buf.0.as_mut_ptr() as usize + 512;
        // SAFETY: `buf` is an unused 512-byte, 16-aligned buffer.
        let f = unsafe { init_frame(top, 0x4000, 0x1234, 0x5678, 0x9abc, MSTATUS_MPIE) };
        assert_eq!(f as usize, top - TRAP_FRAME_SIZE);
        // SAFETY: `f` was just initialised by `init_frame`.
        let f = unsafe { &*f };
        assert_eq!(f.mepc, 0x1234);
        assert_eq!(f.regs[REG_RA], 0x5678);
        assert_eq!(f.regs[REG_GP], 0x9abc);
        assert_eq!(f.regs[REG_SP], 0x4000);
        assert_eq!(f.mstatus, MSTATUS_MPIE);
    }
}

#[cfg(test)]
mod kernel_stack_tests {
    use super::*;

    #[test]
    fn kernel_stacks_are_disjoint_and_aligned() {
        for slot in 0..crate::config::MAX_TASKS {
            let top = kernel_stack_top(slot);
            assert_eq!(top % 16, 0);
            if slot > 0 {
                assert_eq!(top - kernel_stack_top(slot - 1), KERNEL_STACK_SIZE);
            }
        }
    }
}
