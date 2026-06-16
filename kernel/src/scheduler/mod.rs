//! CA-PIP preemptive scheduler for FerretOS.
//!
//! # Architecture
//!
//! 1. **Build-time CCG + MIP** — `build.rs` parses `src/generated/demo_tasks.rs`,
//!    constructs the CCG, and runs BFS (with alloc, on the host) to produce
//!    `src/generated/ccg_constants.rs`.  Zero graph traversal happens at boot.
//! 2. **MIP write** (`write_precomputed_mip`) — O(N) copy from the const array
//!    into each `TaskDescriptor.max_inherited_priority`.
//! 3. **Ready queue** (`queue.rs`) — static max-heap keyed by `effective_priority`.
//! 4. **Preemption check** (this file) — called from the timer ISR.  Compares the
//!    running task's `effective_priority` with the head of the ready queue; triggers
//!    a context switch when a higher-priority task becomes the head.
//!
//! `ccg.rs` and `mip.rs` are retained for their unit-test coverage but are not
//! called at boot.
//!
//! # CA-PIP guarantee
//!
//! Because `max_inherited_priority` is precomputed and never changes at runtime,
//! priority inversion is structurally impossible: a holder's effective priority
//! is always ≥ every waiter's priority, so it can never be preempted by a waiter.

pub mod ccg;
pub mod mip;
pub mod queue;

#[cfg(test)]
mod tests;

use crate::config::MAX_TASKS;
use crate::generated::ccg_constants::MAX_INHERITED_PRIORITIES;
use crate::memory::task::{registry, task_count, task_registry_ptr, TaskDescriptor};
use queue::PriorityQueue;

// ccg and mip modules are retained for their test coverage; they are no
// longer called at boot.
#[allow(unused_imports)]
use ccg::CapabilityContentionGraph;
#[allow(unused_imports)]
use mip::compute_and_store_mip;

/// Time quantum in timer ticks (1 tick = 1 ms by default).
///
/// A task that has held the CPU for this many ticks without yielding is moved
/// to the back of its priority level (round-robin tie-breaking).
pub const TIME_QUANTUM_TICKS: u32 = 5;

/// Maximum context switch latency in CLINT cycles before a warning is logged.
///
/// At 10 MHz that is 100 µs.  Exceeding this suggests the trap entry/exit path
/// has grown beyond the Sprint 4 budget.
pub const SWITCH_LATENCY_WARN_CYCLES: u32 = 1_000;

/// Number of context switches to accumulate before printing latency stats.
const SWITCH_STATS_WINDOW: u32 = 1_000;

// ---------------------------------------------------------------------------
// Context-switch latency statistics (Issue #41)
// ---------------------------------------------------------------------------

/// Running min/max/total statistics over a sliding window of context switches.
///
/// After `SWITCH_STATS_WINDOW` samples the stats are printed to UART and reset.
struct SwitchStats {
    min: u32,
    max: u32,
    total: u64,
    count: u32,
}

impl SwitchStats {
    const fn new() -> Self {
        SwitchStats { min: u32::MAX, max: 0, total: 0, count: 0 }
    }

    /// Record one context-switch latency sample in CLINT cycles.
    fn record(&mut self, cycles: u32) {
        if cycles < self.min { self.min = cycles; }
        if cycles > self.max { self.max = cycles; }
        self.total += cycles as u64;
        self.count += 1;

        if self.count >= SWITCH_STATS_WINDOW {
            let avg = (self.total / self.count as u64) as u32;
            #[cfg(not(any(test, kani)))]
            {
                use crate::uart;
                uart::uart_puts("Switch latency: min=");
                uart::uart_print_usize(self.min as usize);
                uart::uart_puts(" max=");
                uart::uart_print_usize(self.max as usize);
                uart::uart_puts(" avg=");
                uart::uart_print_usize(avg as usize);
                uart::uart_puts(" cycles");
                if self.max > SWITCH_LATENCY_WARN_CYCLES {
                    uart::uart_puts(" [WARN: exceeded budget]");
                }
                uart::uart_puts("\n");
            }
            let _ = avg; // suppress unused warning in test builds
            // Reset for next window.
            *self = SwitchStats::new();
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler state — populated at boot, used by the timer ISR.
// ---------------------------------------------------------------------------

/// Global ready queue — max-heap keyed by effective priority.
static mut READY_QUEUE: PriorityQueue<MAX_TASKS> = PriorityQueue::new();

/// Registry index of the currently running task (0xFF = none / idle).
static mut CURRENT_TASK_IDX: u8 = 0xFF;

/// Number of ticks the current task has run without being preempted.
static mut TICKS_SINCE_SWITCH: u32 = 0;

/// Context-switch latency statistics accumulator.
static mut SWITCH_STATS: SwitchStats = SwitchStats::new();

// ---------------------------------------------------------------------------
// Boot-time initialisation
// ---------------------------------------------------------------------------

/// Write precomputed MIP values from `ccg_constants` into the task registry.
///
/// build.rs parsed demo_tasks.rs, built the CCG, and ran BFS on the host
/// (with alloc) to populate `MAX_INHERITED_PRIORITIES`.  This call simply
/// copies those values — O(N) writes, zero graph traversal at boot.
///
/// # Safety invariant
///
/// Must be called on the single-threaded boot path before interrupts fire.
fn write_precomputed_mip() {
    let n = task_count();
    // SAFETY: single-threaded boot path, no concurrent writes.
    unsafe {
        let reg_ptr: *mut [Option<TaskDescriptor>; MAX_TASKS] = task_registry_ptr();
        for i in 0..n {
            if let Some(slot) = (*reg_ptr)[i].as_mut() {
                slot.max_inherited_priority = MAX_INHERITED_PRIORITIES[i];
            }
        }
    }
}

/// Initialise the scheduler subsystem.
///
/// 1. Writes precomputed MIP into every `TaskDescriptor` (from build.rs constants).
/// 2. Enqueues all `Ready` tasks into the ready queue.
/// 3. Sets the highest-priority task as the current task.
///
/// # Safety invariant
///
/// Must be called after all tasks are registered and before interrupts are
/// enabled (single-threaded boot path).
pub fn init() {
    write_precomputed_mip();

    // Read registry after MIP has been written.
    let reg = registry();

    // SAFETY: single-threaded boot path; no ISR can fire yet.
    // addr_of_mut! avoids creating a Rust reference to the mutable statics,
    // which is UB-prone under the Rust 2024 static-mut-refs rules.
    unsafe {
        let rq = core::ptr::addr_of_mut!(READY_QUEUE);
        for (i, slot) in reg.iter().enumerate() {
            if let Some(task) = slot {
                if task.state == crate::memory::task::TaskState::Ready {
                    (*rq).insert(i as u8, task.effective_priority());
                }
            }
        }

        // The head of the queue becomes the initially running task.
        if let Some(idx) = (*rq).pop_max() {
            *core::ptr::addr_of_mut!(CURRENT_TASK_IDX) = idx;
            *core::ptr::addr_of_mut!(TICKS_SINCE_SWITCH) = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime preemption (called from timer ISR)
// ---------------------------------------------------------------------------

/// Preemption check — called on every timer tick from the machine-timer ISR.
///
/// Returns `Some(new_task_idx)` when the scheduler decides to switch away from
/// the current task, `None` when the current task continues running.
///
/// The caller (trap handler) is responsible for invoking `__context_switch`
/// when `Some` is returned.
///
/// # Arguments
///
/// * `ticks` — `mtime` value sampled at ISR entry; used to measure the
///   preemption-decision latency for Issue #41 statistics.
///
/// # Safety invariant
///
/// Must only be called from the machine-mode trap handler (M-mode, interrupts
/// disabled at entry).  READY_QUEUE and CURRENT_TASK_IDX are accessed without
/// a lock; correctness relies on single-hart, interrupt-disabled execution.
pub fn tick(ticks: u32) -> Option<u8> {
    // SAFETY: called from M-mode trap handler with interrupts disabled.
    // Single-hart; no concurrent access to scheduler state.
    // addr_of_mut! avoids creating Rust references to mutable statics
    // (UB-prone under the Rust 2024 static-mut-refs rules).
    unsafe {
        let ticks_ptr = core::ptr::addr_of_mut!(TICKS_SINCE_SWITCH);
        let cur_ptr   = core::ptr::addr_of_mut!(CURRENT_TASK_IDX);
        let rq        = core::ptr::addr_of_mut!(READY_QUEUE);

        *ticks_ptr = (*ticks_ptr).wrapping_add(1);

        let cur_idx = *cur_ptr as usize;
        let reg = registry();

        // Determine effective priority of the running task.
        let cur_eff = reg
            .get(cur_idx)
            .and_then(|s| s.as_ref())
            .map_or(0, |t| t.effective_priority());

        // Determine effective priority of the best ready task.
        let next_eff = (*rq).peek_max_priority();

        // Preempt if a strictly higher-priority task is waiting, OR if the
        // quantum has expired and a same-or-higher priority task is waiting.
        let quantum_expired = *ticks_ptr >= TIME_QUANTUM_TICKS;
        let should_switch =
            next_eff > cur_eff
            || (quantum_expired && next_eff >= cur_eff && !(*rq).is_empty());

        if should_switch {
            // Move the current task back to the ready queue.
            if let Some(t) = reg.get(cur_idx).and_then(|s| s.as_ref()) {
                (*rq).insert(*cur_ptr, t.effective_priority());
            }

            // Pop the new task.
            if let Some(next_idx) = (*rq).pop_max() {
                let old_idx = *cur_ptr;
                *cur_ptr = next_idx;
                *ticks_ptr = 0;

                if next_idx != old_idx {
                    // Measure the decision latency from ISR entry (ticks is
                    // the mtime value sampled by the caller at ISR entry).
                    let now = crate::clint::get_mtime() as u32;
                    let elapsed = now.wrapping_sub(ticks);
                    // SAFETY: SWITCH_STATS accessed only in M-mode ISR with
                    // interrupts disabled; no concurrent access possible.
                    let stats = core::ptr::addr_of_mut!(SWITCH_STATS);
                    (*stats).record(elapsed);

                    return Some(next_idx);
                }
            }
        }

        None
    }
}

/// Return the registry index of the currently running task.
pub fn current_task_idx() -> u8 {
    // SAFETY: read of a single byte via raw pointer; no torn read possible on RV32.
    unsafe { *core::ptr::addr_of!(CURRENT_TASK_IDX) }
}
