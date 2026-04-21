//! CA-PIP preemptive scheduler for FerretOS.
//!
//! # Architecture
//!
//! 1. **CCG construction** (`ccg.rs`) — built once at boot from task descriptors.
//! 2. **MIP computation** (`mip.rs`) — BFS over CCG; writes `max_inherited_priority`
//!    into each `TaskDescriptor`.  Also runs once at boot.
//! 3. **Ready queue** (`queue.rs`) — static max-heap keyed by `effective_priority`.
//! 4. **Preemption check** (this file) — called from the timer ISR.  Compares the
//!    running task's `effective_priority` with the head of the ready queue; triggers
//!    a context switch when a higher-priority task becomes the head.
//!
//! # CA-PIP guarantee
//!
//! Because `max_inherited_priority` is precomputed and never changes at runtime,
//! priority inversion is structurally impossible: a holder's effective priority
//! is always ≥ every waiter's priority, so it can never be preempted by a waiter.

pub mod ccg;
pub mod mip;
pub mod queue;

use crate::config::MAX_TASKS;
use crate::memory::task::registry;
use ccg::CapabilityContentionGraph;
use mip::compute_and_store_mip;
use queue::PriorityQueue;

// Re-export key symbols used by main.rs and the timer ISR.
pub use mip::compute_and_store_mip as init_mip;

/// Time quantum in timer ticks (1 tick = 1 ms by default).
///
/// A task that has held the CPU for this many ticks without yielding is moved
/// to the back of its priority level (round-robin tie-breaking).
pub const TIME_QUANTUM_TICKS: u32 = 5;

// ---------------------------------------------------------------------------
// Scheduler state — populated at boot, used by the timer ISR.
// ---------------------------------------------------------------------------

/// Global ready queue — max-heap keyed by effective priority.
static mut READY_QUEUE: PriorityQueue<MAX_TASKS> = PriorityQueue::new();

/// Registry index of the currently running task (0xFF = none / idle).
static mut CURRENT_TASK_IDX: u8 = 0xFF;

/// Number of ticks the current task has run without being preempted.
static mut TICKS_SINCE_SWITCH: u32 = 0;

// ---------------------------------------------------------------------------
// Boot-time initialisation
// ---------------------------------------------------------------------------

/// Initialise the scheduler subsystem.
///
/// 1. Builds the CCG from the task registry.
/// 2. Computes and stores MIP in every `TaskDescriptor`.
/// 3. Enqueues all `Ready` tasks into the ready queue.
/// 4. Sets the highest-priority task as the current task.
///
/// # Safety invariant
///
/// Must be called after all tasks are registered and before interrupts are
/// enabled (single-threaded boot path).
pub fn init() {
    let reg = registry();
    let ccg = CapabilityContentionGraph::build(reg);
    compute_and_store_mip(&ccg);

    // Re-read registry after MIP has been written back.
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
/// * `ticks` — current absolute tick count (used for time-quantum enforcement).
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
                let _ = ticks; // available for latency logging (Issue #41)
                if next_idx != old_idx {
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
