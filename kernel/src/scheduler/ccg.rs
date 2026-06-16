//! Capability Contention Graph (CCG) for CA-PIP scheduling.
//!
//! The CCG is a directed graph over tasks.  An edge L → H means task L holds
//! an exclusive peripheral that task H requires: `L.exclusive_cap_mask &
//! H.required_cap_mask != 0`.
//!
//! This formulation separates *holding* (`exclusive_cap_mask`) from *needing*
//! (`required_cap_mask`), so the boot-time conflict detector — which halts if
//! two tasks both hold the same exclusive peripheral — does not falsely trigger
//! on valid holder–waiter relationships.
//!
//! # Complexity
//!
//! O(N²) construction, executed once at boot.  N ≤ MAX_TASKS = 16.

use crate::config::MAX_TASKS;
use crate::memory::task::TaskDescriptor;

/// Directed adjacency-matrix representation of the CCG.
///
/// `edges[i][j] == true` means task at registry index `i` holds a capability
/// that the task at registry index `j` requires.
pub struct CapabilityContentionGraph {
    edges: [[bool; MAX_TASKS]; MAX_TASKS],
    /// Number of tasks present in the registry used during construction.
    pub task_count: usize,
}

impl CapabilityContentionGraph {
    /// Build the CCG from the static task registry.
    ///
    /// Iterates over all (L, H) pairs where `L != H` and sets
    /// `edges[L_idx][H_idx]` when `L.exclusive_cap_mask & H.required_cap_mask != 0`.
    ///
    /// # Arguments
    ///
    /// * `registry` — reference to the global `TASK_REGISTRY` array.
    pub fn build(registry: &[Option<TaskDescriptor>; MAX_TASKS]) -> Self {
        let mut ccg = CapabilityContentionGraph {
            edges: [[false; MAX_TASKS]; MAX_TASKS],
            task_count: 0,
        };

        // Collect valid indices so we only iterate over populated slots.
        let mut indices = [0usize; MAX_TASKS];
        let mut count = 0usize;
        for (i, slot) in registry.iter().enumerate() {
            if slot.is_some() {
                indices[count] = i;
                count += 1;
            }
        }
        ccg.task_count = count;

        for (li, &l_idx) in indices[..count].iter().enumerate() {
            // SAFETY: slot was confirmed Some above.
            let l = registry[l_idx].as_ref().unwrap();
            for (hi, &h_idx) in indices[..count].iter().enumerate() {
                if li == hi {
                    continue;
                }
                let h = registry[h_idx].as_ref().unwrap();
                // Edge L → H: L holds something that H needs.
                if (l.exclusive_cap_mask & h.required_cap_mask) != 0 {
                    ccg.edges[l_idx][h_idx] = true;
                }
            }
        }

        ccg
    }

    /// Returns `true` if there is a direct CCG edge from registry index `from`
    /// to registry index `to`.
    pub fn has_edge(&self, from: usize, to: usize) -> bool {
        from < MAX_TASKS && to < MAX_TASKS && self.edges[from][to]
    }

    /// Iterate over the registry indices of all successors of `from`.
    pub fn successors(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        (0..MAX_TASKS).filter(move |&j| self.edges[from][j])
    }

    /// Detect cycles in the CCG using iterative DFS; return `true` if any cycle exists.
    ///
    /// A CCG cycle (A holds cap0 needed by B; B holds cap1 needed by A) means the
    /// two tasks are in a mutual-blocking relationship.  The MIP BFS handles cycles
    /// correctly via `visited[]`, but the presence of a cycle is almost always a
    /// declaration error worth surfacing at boot.
    ///
    /// Algorithm: for each unvisited node, push it onto a DFS stack.  Track a
    /// `rec_stack` (nodes currently on the recursion path); a back-edge to a node
    /// in `rec_stack` means a cycle.  O(N²) time, O(N) space.
    ///
    /// Returns the index pair `(u, v)` of the back-edge that closes the first
    /// detected cycle, or `None` if the graph is acyclic.
    pub fn detect_cycle(&self) -> Option<(usize, usize)> {
        let n = self.task_count;
        let mut visited  = [false; MAX_TASKS];
        let mut rec_stack = [false; MAX_TASKS];

        for start in 0..n {
            if visited[start] {
                continue;
            }
            // Iterative DFS using an explicit stack of (node, iterator-state).
            // iterator-state is the next successor index to explore from `node`.
            let mut dfs: [(usize, usize); MAX_TASKS] = [(0, 0); MAX_TASKS];
            let mut depth = 0;
            dfs[0] = (start, 0);
            visited[start] = true;
            rec_stack[start] = true;

            loop {
                let (node, next_succ) = dfs[depth];
                // Find the next unvisited (or in-rec-stack) successor.
                let mut found = false;
                for succ in next_succ..MAX_TASKS {
                    if !self.edges[node][succ] {
                        continue;
                    }
                    // Update iterator state for next iteration at this depth.
                    dfs[depth].1 = succ + 1;
                    if rec_stack[succ] {
                        // Back-edge found — cycle exists.
                        return Some((node, succ));
                    }
                    if !visited[succ] {
                        visited[succ] = true;
                        rec_stack[succ] = true;
                        depth += 1;
                        dfs[depth] = (succ, 0);
                    }
                    found = true;
                    break;
                }
                if !found {
                    // All successors of `node` exhausted — backtrack.
                    rec_stack[node] = false;
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
            }
        }
        None
    }
}
