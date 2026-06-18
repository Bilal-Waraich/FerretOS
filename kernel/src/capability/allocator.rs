//! Boot-time capability conflict detector.
//!
//! Scans the task registry for exclusive capability conflicts before the
//! scheduler starts.  If any two tasks claim the same exclusive peripheral ID,
//! the kernel prints a diagnostic and halts permanently — task execution must
//! never begin with a violated capability invariant.
//!
//! # Algorithm
//!
//! Iterate over all registered tasks; for each task scan the 32 bits of
//! `exclusive_cap_mask` by index.  Track which peripheral IDs have already been
//! claimed in a fixed-size `claimed` array.  O(N × P) where N ≤ MAX_TASKS = 16
//! and P ≤ MAX_PERIPHERALS = 32 — effectively O(1) for this target.
//!
//! The scan uses an explicit `0..32` index loop rather than the
//! `exclusive_capabilities()` adapter so that the `claimed[cap_id]` access uses
//! a concrete index.  This keeps the Kani harness tractable: an index that
//! flows through an iterator adapter is treated as symbolic by the model
//! checker, turning every array write into a 32-way array-theory update.

use crate::config::MAX_PERIPHERALS;
use crate::memory::task::TaskDescriptor;
#[cfg(not(any(test, kani)))]
use crate::uart;

/// Check for exclusive capability conflicts across all registered tasks.
///
/// # Panics / halts
///
/// If a conflict is found, prints a UART diagnostic and spins forever.
/// Does not return in that case.
///
/// # Arguments
///
/// * `registry` — slice of `Option<TaskDescriptor>` from the task registry.
pub fn check_capability_conflicts(registry: &[Option<TaskDescriptor>]) {
    // claimed[i] = Some(task_id) means peripheral i is already held exclusively.
    let mut claimed: [Option<u8>; MAX_PERIPHERALS] = [None; MAX_PERIPHERALS];

    for slot in registry {
        let Some(task) = slot else { continue };
        // Scan every bit of the u32 mask with a concrete index so `claimed`
        // is accessed by a constant offset (see module docs).
        for cap_id in 0..u32::BITS as usize {
            if task.exclusive_cap_mask & (1u32 << cap_id) == 0 {
                continue;
            }
            if cap_id >= MAX_PERIPHERALS {
                // Bit set beyond the supported range — treat as configuration error.
                report_out_of_range_and_halt(cap_id, task.id);
            }
            if let Some(prior_id) = claimed[cap_id] {
                report_conflict_and_halt(cap_id, prior_id, task.id);
            }
            claimed[cap_id] = Some(task.id);
        }
    }
}

/// Print a conflict diagnostic to UART and spin forever.
///
/// Called when two tasks claim the same exclusive peripheral.
/// Never returns.
fn report_conflict_and_halt(cap_id: usize, task_a: u8, task_b: u8) -> ! {
    // In test builds, panic with a message so #[should_panic] tests work.
    // In kernel builds, print to UART and spin — panic_handler calls loop {}.
    #[cfg(not(any(test, kani)))]
    {
        uart::uart_puts("CAPABILITY CONFLICT: peripheral ");
        uart::uart_print_usize(cap_id);
        uart::uart_puts(" claimed by task ");
        uart::uart_print_usize(task_a as usize);
        uart::uart_puts(" and task ");
        uart::uart_print_usize(task_b as usize);
        uart::uart_puts("\n");
        uart::uart_puts("System halted. Fix capability declarations and reboot.\n");
        // Spin with wfi so the core yields to any pending interrupt rather than
        // burning power, while still never returning.
        loop {
            // SAFETY: wfi is always safe to execute in M-mode; we are halting
            // intentionally after a fatal capability conflict.
            unsafe { core::arch::asm!("wfi") };
        }
    }
    #[cfg(any(test, kani))]
    panic!(
        "CAPABILITY CONFLICT: peripheral {} claimed by task {} and task {}",
        cap_id, task_a, task_b
    );
}

/// Print an out-of-range diagnostic to UART and spin forever.
///
/// Called when a capability ID exceeds MAX_PERIPHERALS.
/// Never returns.
fn report_out_of_range_and_halt(cap_id: usize, task_id: u8) -> ! {
    #[cfg(not(any(test, kani)))]
    {
        uart::uart_puts("CAPABILITY ERROR: peripheral ID ");
        uart::uart_print_usize(cap_id);
        uart::uart_puts(" out of range (MAX_PERIPHERALS=");
        uart::uart_print_usize(MAX_PERIPHERALS);
        uart::uart_puts(") in task ");
        uart::uart_print_usize(task_id as usize);
        uart::uart_puts("\n");
        uart::uart_puts("System halted. Fix capability declarations and reboot.\n");
        loop {
            // SAFETY: wfi is always safe to execute in M-mode; we are halting
            // intentionally after a fatal configuration error.
            unsafe { core::arch::asm!("wfi") };
        }
    }
    #[cfg(any(test, kani))]
    panic!(
        "CAPABILITY ERROR: peripheral ID {} out of range in task {}",
        cap_id, task_id
    );
}
