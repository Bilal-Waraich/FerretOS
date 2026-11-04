//! MaxInheritedPriority (MIP) computation via BFS over the CCG.
//!
//! For each task T, `MIP(T)` is the maximum base priority among all tasks
//! reachable from T by following CCG edges (L → H means L can be priority-
//! boosted because H needs what L holds).
//!
//! `effective_priority(T) = max(T.priority, MIP(T))`
//!
//! MIP is precomputed once at boot and stored in
//! `TaskDescriptor.max_inherited_priority`.  All scheduler decisions at
//! runtime are therefore two integer comparisons — O(1).
//!
//! # Complexity
//!
//! O(N²) BFS from each node; N ≤ MAX_TASKS = 16.  Runs once at boot.

use crate::config::MAX_TASKS;
use crate::memory::task::{registry, task_count, task_registry_ptr, TaskDescriptor};
use crate::scheduler::ccg::CapabilityContentionGraph;

/// Compute MIP for every occupied slot of `reg` without touching global state.
///
/// `result[i]` is the highest base priority of any task reachable from slot
/// `i` via the CCG, including `i` itself (so `MIP ≥ priority`); empty slots
/// are 0.  O(N²) per source, O(N³) total, N ≤ MAX_TASKS.
pub fn compute_mip_values(
    ccg: &CapabilityContentionGraph,
    reg: &[Option<TaskDescriptor>; MAX_TASKS],
) -> [u8; MAX_TASKS] {
    let mut mip = [0u8; MAX_TASKS];

    for src in 0..MAX_TASKS {
        let Some(src_task) = &reg[src] else { continue };
        mip[src] = src_task.priority;

        // BFS queue — fixed-size array; each slot is enqueued at most once.
        let mut queue = [0usize; MAX_TASKS];
        let mut visited = [false; MAX_TASKS];
        let mut head = 0usize;
        let mut tail = 0usize;

        queue[tail] = src;
        tail += 1;
        visited[src] = true;

        while head < tail {
            let cur = queue[head];
            head += 1;

            if let Some(t) = &reg[cur] {
                mip[src] = mip[src].max(t.priority);
            }

            for succ in ccg.successors(cur) {
                if !visited[succ] && reg[succ].is_some() {
                    visited[succ] = true;
                    queue[tail] = succ;
                    tail += 1;
                }
            }
        }
    }
    mip
}

/// Compute MIP for every task and write results back to the task registry.
///
/// After this call, `registry()[i].max_inherited_priority` holds
/// [`compute_mip_values`] for slot `i`.
///
/// # Safety invariant
///
/// Must be called after all tasks are registered and before interrupts are
/// enabled.  Writes to `TASK_REGISTRY` via raw pointer to avoid
/// creating a `&mut` reference to a mutable static (UB-prone under Rust 2024).
pub fn compute_and_store_mip(ccg: &CapabilityContentionGraph) {
    let mip = compute_mip_values(ccg, registry());

    // SAFETY: called on the single-threaded boot path before interrupts are
    // enabled.  No other code aliases TASK_REGISTRY at this point.
    unsafe {
        let reg_ptr: *mut [Option<TaskDescriptor>; MAX_TASKS] = task_registry_ptr();
        for (i, m) in mip.iter().enumerate().take(task_count()) {
            if let Some(slot) = (*reg_ptr)[i].as_mut() {
                slot.max_inherited_priority = *m;
            }
        }
    }
}
