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
use crate::memory::task::{TaskDescriptor, registry, task_count};
use crate::scheduler::ccg::CapabilityContentionGraph;

/// Compute MIP for every task and write results back to the task registry.
///
/// After this call, `registry()[i].max_inherited_priority` holds the highest
/// base priority of any task reachable from task `i` via the CCG (including
/// `i` itself, so `MIP >= i.priority`).
///
/// # Safety invariant
///
/// Must be called after all tasks are registered and before interrupts are
/// enabled.  Writes to `TASK_REGISTRY` via raw pointer to avoid
/// creating a `&mut` reference to a mutable static (UB-prone under Rust 2024).
pub fn compute_and_store_mip(ccg: &CapabilityContentionGraph) {
    let n = task_count();
    // mip[i] stores the computed MaxInheritedPriority for registry slot i.
    let mut mip = [0u8; MAX_TASKS];

    // Seed each slot with the task's own base priority.
    {
        let reg = registry();
        for (i, slot) in reg.iter().enumerate().take(n) {
            if let Some(t) = slot {
                mip[i] = t.priority;
            }
        }
    }

    // BFS from each source task i; propagate max priority to all reachable tasks.
    // We use a fixed-size visited bitset to avoid re-visiting nodes.
    let reg = registry();
    for src in 0..n {
        if reg[src].is_none() {
            continue;
        }
        // BFS queue — small fixed-size array (MAX_TASKS ≤ 16).
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

            // Propagate the max priority discovered at cur back to src.
            if let Some(t) = &reg[cur] {
                if t.priority > mip[src] {
                    mip[src] = t.priority;
                }
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

    // Write MIP values back into TASK_REGISTRY via raw pointer.
    // SAFETY: called on the single-threaded boot path before interrupts are
    // enabled.  No other code aliases TASK_REGISTRY at this point.
    // addr_of_mut! avoids creating a &mut reference to the mutable static,
    // which is UB-prone under the Rust 2024 static-mut-refs rules.
    unsafe {
        use crate::memory::task::task_registry_ptr;
        let reg_ptr: *mut [Option<TaskDescriptor>; MAX_TASKS] = task_registry_ptr();
        for (i, m) in mip.iter().enumerate().take(n) {
            if let Some(slot) = (*reg_ptr)[i].as_mut() {
                slot.max_inherited_priority = *m;
            }
        }
    }
}
