//! Host-side Capability Contention Graph generator.
//!
//! Shared by `build.rs` (which uses it to emit `src/generated/ccg_constants.rs`)
//! and by the kernel library's host unit tests (`lib.rs` includes this file
//! under `cfg(test)`).  Because the build script is a correctness artifact — a
//! bug here silently produces wrong `MaxInheritedPriority` values with no
//! runtime error — it is tested like kernel code.
//!
//! Runs on the host with `std`; never compiled into the kernel binary.

use std::collections::VecDeque;

/// Scheduling-relevant fields of one generated `TaskConfig` static.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskConfig {
    pub priority: u8,
    pub exclusive_cap_mask: u32,
    pub required_cap_mask: u32,
}

/// Parse every `pub static NAME: TaskConfig = TaskConfig { ... };` block from
/// generated task-set source (e.g. `demo_tasks.rs`), in file order.
///
/// File order must match registration order in `kernel_main`: the index in
/// the returned `Vec` is the task's registry slot.  Missing fields parse as 0.
pub fn parse_task_configs(src: &str) -> Vec<TaskConfig> {
    src.split("pub static")
        .skip(1)
        .filter(|block| block.contains("TaskConfig"))
        .map(|block| TaskConfig {
            priority: parse_u32_field(block, "priority") as u8,
            exclusive_cap_mask: parse_u32_field(block, "exclusive_cap_mask"),
            required_cap_mask: parse_u32_field(block, "required_cap_mask"),
        })
        .collect()
}

/// Extract the value of field `field` from a Rust struct-literal block.
///
/// Handles decimal (`0`, `4096`) and hexadecimal (`0x0000_0001`) literals with
/// underscore separators, and any leading indentation (spaces or tabs).
/// Returns 0 if the field is absent or its value does not parse.
pub fn parse_u32_field(block: &str, field: &str) -> u32 {
    for line in block.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(field) else { continue };
        // `rest` must start the value (`: ...`), not a longer field name that
        // merely begins with `field` (e.g. `priority_class`).
        let Some(rest) = rest.trim_start().strip_prefix(':') else { continue };
        let val = rest.trim().trim_end_matches(',').replace('_', "");
        if let Some(hex) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
            return u32::from_str_radix(hex, 16).unwrap_or(0);
        }
        return val.parse::<u32>().unwrap_or(0);
    }
    0
}

/// Build the CCG adjacency matrix: `edges[l][h]` is true when task `l` holds
/// an exclusive capability that task `h` requires.  O(N²).
pub fn build_edges(tasks: &[TaskConfig]) -> Vec<Vec<bool>> {
    let n = tasks.len();
    let mut edges = vec![vec![false; n]; n];
    for (l, tl) in tasks.iter().enumerate() {
        for (h, th) in tasks.iter().enumerate() {
            if l != h && (tl.exclusive_cap_mask & th.required_cap_mask) != 0 {
                edges[l][h] = true;
            }
        }
    }
    edges
}

/// Compute `MaxInheritedPriority` for every task by BFS over the CCG.
///
/// `mip[i]` is the maximum base priority over all tasks reachable from task
/// `i`, including `i` itself.  O(N³) total (one O(N²) BFS per task).
pub fn compute_mip(tasks: &[TaskConfig]) -> Vec<u8> {
    let n = tasks.len();
    let edges = build_edges(tasks);
    let mut mip: Vec<u8> = tasks.iter().map(|t| t.priority).collect();

    for src in 0..n {
        let mut visited = vec![false; n];
        let mut queue = VecDeque::new();
        queue.push_back(src);
        visited[src] = true;
        while let Some(cur) = queue.pop_front() {
            mip[src] = mip[src].max(tasks[cur].priority);
            for (succ, &is_edge) in edges[cur].iter().enumerate() {
                if is_edge && !visited[succ] {
                    visited[succ] = true;
                    queue.push_back(succ);
                }
            }
        }
    }
    mip
}

/// Return every DFS back edge `(from, to)` in the CCG; non-empty iff the CCG
/// has a cycle (a mutual block between tasks).
pub fn find_cycle_back_edges(tasks: &[TaskConfig]) -> Vec<(usize, usize)> {
    fn dfs(
        u: usize,
        edges: &[Vec<bool>],
        visited: &mut [bool],
        on_stack: &mut [bool],
        out: &mut Vec<(usize, usize)>,
    ) {
        visited[u] = true;
        on_stack[u] = true;
        for v in 0..edges[u].len() {
            if !edges[u][v] {
                continue;
            }
            if !visited[v] {
                dfs(v, edges, visited, on_stack, out);
            } else if on_stack[v] {
                out.push((u, v));
            }
        }
        on_stack[u] = false;
    }

    let n = tasks.len();
    let edges = build_edges(tasks);
    let mut visited = vec![false; n];
    let mut on_stack = vec![false; n];
    let mut out = Vec::new();
    for start in 0..n {
        if !visited[start] {
            dfs(start, &edges, &mut visited, &mut on_stack, &mut out);
        }
    }
    out
}

/// One task set's precomputed priorities, for [`render_ccg_constants`].
pub struct MipTable<'a> {
    /// Name of the emitted `const`, e.g. `HML_MAX_INHERITED_PRIORITIES`.
    pub const_name: &'a str,
    /// Generated file the task set was parsed from, for the comment.
    pub source: &'a str,
    /// `MaxInheritedPriority` per registry slot, in file order.
    pub mip: &'a [u8],
}

/// Render `ccg_constants.rs`: one `[u8; max_tasks]` array of precomputed MIP
/// values per task set, indexed by registry slot and zero-padded past the
/// last task.
pub fn render_ccg_constants(tables: &[MipTable<'_>], max_tasks: usize) -> String {
    let mut out = String::from(
        "// Auto-generated by kernel/build.rs — do not edit manually.\n\
         // Rebuild after editing tasks/*.oml.\n\
         //\n\
         // Each array holds MaxInheritedPriority for every task registry slot of\n\
         // one task set, computed via BFS over the Capability Contention Graph at\n\
         // build time.  The boot code passes the array for the selected demo to\n\
         // scheduler::init, which writes the values into each TaskDescriptor\n\
         // instead of running the BFS at runtime.  Slot order is the order of\n\
         // the instances in the source file, which is the registration order.\n",
    );
    for t in tables {
        let mut array = vec![0u8; max_tasks];
        for (slot, &m) in array.iter_mut().zip(t.mip) {
            *slot = m;
        }
        let entries: Vec<String> = array.iter().map(|v| v.to_string()).collect();
        out.push_str(&format!(
            "\n/// From `src/generated/{source}`.\npub const {name}: [u8; {max_tasks}] = [{entries}];\n",
            source = t.source,
            name = t.const_name,
            max_tasks = max_tasks,
            entries = entries.join(", "),
        ));
    }
    out
}

/// Task sets whose priorities `build.rs` precomputes: (generated file, const).
pub const TASK_SETS: [(&str, &str); 2] = [
    ("demo_tasks.rs", "HML_MAX_INHERITED_PRIORITIES"),
    ("pipeline_tasks.rs", "PIPELINE_MAX_INHERITED_PRIORITIES"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn t(priority: u8, exclusive: u32, required: u32) -> TaskConfig {
        TaskConfig { priority, exclusive_cap_mask: exclusive, required_cap_mask: required }
    }

    const UART0: u32 = 1 << 0;

    #[test]
    fn empty_task_set() {
        assert!(compute_mip(&[]).is_empty());
        assert!(find_cycle_back_edges(&[]).is_empty());
    }

    #[test]
    fn single_task_mip_is_own_priority() {
        assert_eq!(compute_mip(&[t(4, UART0, UART0)]), vec![4]);
    }

    #[test]
    fn demo_h_m_l() {
        // L holds UART0, M has nothing, H requires UART0.
        let tasks = [t(1, UART0, 0), t(2, 0, 0), t(3, 0, UART0)];
        let edges = build_edges(&tasks);
        assert!(edges[0][2], "edge L -> H");
        assert!(!edges[2][0] && !edges[0][1] && !edges[1][2]);
        assert_eq!(compute_mip(&tasks), vec![3, 2, 3]);
        assert!(find_cycle_back_edges(&tasks).is_empty());
    }

    #[test]
    fn chain_inherits_transitively() {
        // A -> B -> C: A holds what B needs, B holds what C needs.
        let tasks = [t(1, 1 << 0, 0), t(2, 1 << 1, 1 << 0), t(9, 0, 1 << 1)];
        assert_eq!(compute_mip(&tasks), vec![9, 9, 9]);
    }

    #[test]
    fn diamond_reaches_sink_once() {
        // A -> B, A -> C, B -> D, C -> D.
        let tasks = [
            t(1, 1 << 0, 0),
            t(2, 1 << 1, 1 << 0),
            t(3, 1 << 2, 1 << 0),
            t(7, 0, (1 << 1) | (1 << 2)),
        ];
        assert_eq!(compute_mip(&tasks), vec![7, 7, 7, 7]);
    }

    #[test]
    fn unrelated_tasks_keep_own_priority() {
        let tasks = [t(5, 1 << 0, 0), t(1, 1 << 1, 0), t(3, 0, 1 << 2)];
        assert_eq!(compute_mip(&tasks), vec![5, 1, 3]);
    }

    #[test]
    fn mip_never_below_own_priority() {
        // A high-priority holder is not lowered by a low-priority waiter.
        let tasks = [t(8, UART0, 0), t(1, 0, UART0)];
        assert_eq!(compute_mip(&tasks), vec![8, 1]);
    }

    #[test]
    fn two_cycle_is_reported_and_bfs_terminates() {
        let tasks = [t(2, 1 << 0, 1 << 1), t(4, 1 << 1, 1 << 0)];
        assert_eq!(compute_mip(&tasks), vec![4, 4]);
        assert_eq!(find_cycle_back_edges(&tasks).len(), 1);
    }

    #[test]
    fn parses_decimal_hex_and_tab_indented_fields() {
        let src = "pub static TASK_A: TaskConfig = TaskConfig {\n\
                   \tpriority: 7,\n\
                   \texclusive_cap_mask: 0x0000_0003,\n\
                   \trequired_cap_mask: 12,\n\
                   };\n\
                   pub static OTHER: u32 = 5;\n\
                   pub static TASK_B: TaskConfig = TaskConfig {\n    priority: 0x10,\n};\n";
        assert_eq!(parse_task_configs(src), vec![t(7, 3, 12), t(16, 0, 0)]);
    }

    #[test]
    fn field_prefix_does_not_match_longer_name() {
        let block = "    priority_class: 9,\n    priority: 2,\n";
        assert_eq!(parse_u32_field(block, "priority"), 2);
    }

    #[test]
    fn render_pads_to_max_tasks() {
        let out = render_ccg_constants(&[MipTable { const_name: "X", source: "x.rs", mip: &[3, 2, 3] }], 6);
        assert!(out.ends_with("pub const X: [u8; 6] = [3, 2, 3, 0, 0, 0];\n"));
    }

    // --- Consistency with the committed and runtime artefacts -------------

    #[test]
    fn committed_ccg_constants_match_generator() {
        // Regenerating from the committed task sets must reproduce the
        // committed ccg_constants.rs byte for byte.
        let hml = compute_mip(&parse_task_configs(include_str!("../src/generated/demo_tasks.rs")));
        let pipe = compute_mip(&parse_task_configs(include_str!("../src/generated/pipeline_tasks.rs")));
        let tables = [
            MipTable { const_name: TASK_SETS[0].1, source: TASK_SETS[0].0, mip: &hml },
            MipTable { const_name: TASK_SETS[1].1, source: TASK_SETS[1].0, mip: &pipe },
        ];
        let committed = include_str!("../src/generated/ccg_constants.rs");
        assert_eq!(render_ccg_constants(&tables, crate::config::MAX_TASKS), committed);
    }

    #[test]
    fn generator_agrees_with_in_kernel_algorithm() {
        // The kernel's CapabilityContentionGraph + compute_mip_values are the
        // reference implementation; the build-time generator must agree on
        // every task set exercised here.
        use crate::memory::task::TaskDescriptor;
        use crate::scheduler::{ccg::CapabilityContentionGraph, mip::compute_mip_values};

        let sets: [&[TaskConfig]; 5] = [
            &[t(1, UART0, 0), t(2, 0, 0), t(3, 0, UART0)],
            &[t(1, 1 << 0, 0), t(2, 1 << 1, 1 << 0), t(9, 0, 1 << 1)],
            &[t(2, 1 << 0, 1 << 1), t(4, 1 << 1, 1 << 0)],
            &[t(5, 1 << 0, 0), t(1, 1 << 1, 0), t(3, 0, 1 << 2)],
            &[t(1, 1 << 0, 0), t(2, 1 << 1, 1 << 0), t(3, 1 << 2, 1 << 0), t(7, 0, 6)],
        ];
        for tasks in sets {
            let mut reg = [const { None }; crate::config::MAX_TASKS];
            for (i, tc) in tasks.iter().enumerate() {
                reg[i] = Some(TaskDescriptor::with_capabilities(
                    i as u8, tc.priority, 0, 16, 0, 0, tc.exclusive_cap_mask, 0, tc.required_cap_mask,
                ));
            }
            let ccg = CapabilityContentionGraph::build(&reg);
            let kernel = compute_mip_values(&ccg, &reg);
            assert_eq!(&kernel[..tasks.len()], &compute_mip(tasks)[..], "task set {tasks:?}");
        }
    }
}
