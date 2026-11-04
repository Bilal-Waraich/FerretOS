//! Demo task sets and kernel-owned helper tasks.
//!
//! Exactly one task set is registered at boot:
//!
//! - [`pipeline`] (default) — the sensor-fusion pipeline: tasks exchanging
//!   data over capability-gated channels under CA-PIP scheduling.
//! - [`hml`] (`--features demo-hml`) — the CA-PIP H/M/L demo, whose boot
//!   test checks the priority-inheritance ordering property directly.
//!
//! This module also holds the idle task, the return trap installed in every
//! task frame, and the `fault-demo` rogue task.

#[cfg(feature = "demo-hml")]
pub mod hml;
#[cfg(not(feature = "demo-hml"))]
pub mod pipeline;

use ferret::sys::{self, SysError};

/// Prepare a task stack in a TASKRAM `NOLOAD` section: write its canary
/// (the section's initial contents are not loaded) and return its base.
pub fn claim_stack(stack_base: usize) -> usize {
    // SAFETY: boot path, before any task runs; `stack_base` is the address of
    // a `Stack<N>` static that only its task will ever use.
    unsafe { ferret::memory::stack::write_canary(stack_base) };
    stack_base
}

/// Ask the kernel to log the task registry (kernel memory) and report the result.
///
/// Syscall argument validation must refuse with `EFAULT` rather than leak
/// kernel memory; the boot test checks for the reported line.
///
/// The address is only *named* here, never dereferenced by the task.
pub fn probe_log_validation() {
    let kernel_addr = ferret::memory::registry().as_ptr() as usize;
    let msg: &[u8] = match sys::log_raw(kernel_addr, 16) {
        Err(SysError::Fault) => b"[probe] log of kernel memory rejected (EFAULT)\n",
        Err(_) => b"[probe] log of kernel memory rejected with unexpected error\n",
        Ok(_) => b"[probe] log of kernel memory was ACCEPTED\n",
    };
    let _ = sys::log(msg);
}


/// Rogue task for the PMP violation test (feature `fault-demo`).
///
/// After its first time slice it stores to [`ferret::FAULT_PROBE_TARGET`],
/// a kernel variable outside its region.  With `feature = "pmp"` the store
/// raises a store access fault and the kernel halts with a diagnostic; the
/// line after the store is only reached when nothing enforces isolation.
#[cfg(feature = "fault-demo")]
pub fn task_rogue() -> ! {
    sys::sleep(30);
    let target = core::ptr::addr_of_mut!(ferret::FAULT_PROBE_TARGET);
    let _ = ferret::sys::LineBuf::<80>::new()
        .str("[F] writing to kernel memory at ")
        .hex(target as u32)
        .str("\n")
        .log();
    // SAFETY: deliberately unsound — this is the fault injection the PMP
    // test exists to catch.  The target is a kernel-owned word reserved for
    // this probe, so when isolation is off the stray store corrupts nothing
    // else.
    unsafe { core::ptr::write_volatile(target, 0xBAD0_BAD0) };
    let _ = sys::log(b"[F] write to kernel memory SUCCEEDED: no PMP isolation\n");
    loop {
        sys::sleep(1000);
    }
}

/// Idle task — runs when nothing else is ready; waits for the next interrupt.
pub fn idle() -> ! {
    loop {
        // SAFETY: `wfi` only pauses the hart until an interrupt is pending.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Return address installed in every task's initial frame.
///
/// Task bodies are `fn() -> !`, so reaching this is a bug; report it and park
/// the task forever without stopping the rest of the system.
pub fn task_returned() -> ! {
    let _ = sys::log(b"[KERNEL] task returned from its entry point; parking it\n");
    loop {
        sys::sleep(u32::MAX / 2);
    }
}
