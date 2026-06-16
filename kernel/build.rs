// kernel/build.rs — Cargo build script for the ferret kernel crate.
//
// Responsibilities:
//   1. Linker script setup — add kernel/ to the linker search path so riscv-rt
//      can find memory.x (which defines MEMORY regions for our target).
//   2. OML code generation — invoke the OML transpiler on tasks/*.oml to
//      regenerate kernel/src/generated/task_schema.rs.  The build continues
//      without error if OML is not installed; the committed generated files
//      are used instead.
//   3. CCG constant emission — parse src/generated/demo_tasks.rs, compute the
//      Capability Contention Graph and MaxInheritedPriority on the host (with
//      alloc), and write src/generated/ccg_constants.rs.  This eliminates all
//      boot-time graph traversal from the kernel binary.
//
// OML binary resolution order:
//   1. OML_BIN env var (set by CI or developer)
//   2. oml/target/release/oml (git submodule at repo root)
//   3. ../../OML/target/release/oml (sibling repo, local development layout)
//   4. oml on PATH

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_TASKS: usize = 16;

fn main() {
    // --- Linker script setup ------------------------------------------------
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-link-search={}", manifest_dir.display());
    println!("cargo:rerun-if-changed=memory.x");

    // --- OML code generation ------------------------------------------------
    println!("cargo:rerun-if-changed=../tasks/");
    println!("cargo:rerun-if-env-changed=OML_BIN");

    let out_dir = manifest_dir.join("src/generated");
    std::fs::create_dir_all(&out_dir).expect("failed to create src/generated/");

    if let Some(oml_bin) = find_oml_binary(&manifest_dir) {
        let tasks_dir = manifest_dir.join("../tasks");
        if tasks_dir.exists() {
            let task_oml = tasks_dir.join("task.oml");
            if task_oml.exists() {
                let status = Command::new(&oml_bin)
                    .args([
                        task_oml.to_str().unwrap(),
                        "--rust",
                        "--output",
                        out_dir.to_str().unwrap(),
                    ])
                    .status()
                    .expect("failed to invoke OML transpiler");
                assert!(status.success(), "OML transpiler returned non-zero exit code");
            }
        }
    } else {
        println!("cargo:warning=OML binary not found — using committed generated files.");
    }

    // --- CCG constant emission (closes #42) ---------------------------------
    // Always regenerate ccg_constants.rs from the committed demo_tasks.rs so
    // the kernel never has to run CCG construction or MIP BFS at boot.
    println!("cargo:rerun-if-changed=src/generated/demo_tasks.rs");

    let demo_tasks_path = out_dir.join("demo_tasks.rs");
    if demo_tasks_path.exists() {
        let src = std::fs::read_to_string(&demo_tasks_path)
            .expect("failed to read demo_tasks.rs");
        let tasks = parse_task_configs(&src);
        if !tasks.is_empty() {
            let mip = compute_mip(&tasks);
            check_cycles_and_warn(&tasks);
            emit_ccg_constants(&out_dir, &mip);
        }
    }
}

// ---------------------------------------------------------------------------
// Task config parsing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TaskConfig {
    priority: u8,
    exclusive_cap_mask: u32,
    required_cap_mask: u32,
}

/// Parse all `pub static TASK_*: TaskConfig = TaskConfig { ... };` blocks from
/// a generated demo_tasks.rs.  Task order in the file must match registration
/// order in kernel_main — the index into the returned Vec corresponds to the
/// registry slot index written by `register_task`.
fn parse_task_configs(src: &str) -> Vec<TaskConfig> {
    let mut tasks = Vec::new();
    // Split on each `pub static` declaration to get one block per task.
    for block in src.split("pub static").skip(1) {
        // Only process TaskConfig statics.
        if !block.contains("TaskConfig") {
            continue;
        }
        let priority          = parse_u8_field(block, "priority");
        let exclusive_cap_mask = parse_u32_field(block, "exclusive_cap_mask");
        let required_cap_mask  = parse_u32_field(block, "required_cap_mask");
        tasks.push(TaskConfig { priority, exclusive_cap_mask, required_cap_mask });
    }
    tasks
}

fn parse_u8_field(block: &str, field: &str) -> u8 {
    parse_u32_field(block, field) as u8
}

/// Extract the value of a named field from a Rust struct literal block.
///
/// Handles decimal (`0`, `4096`) and hexadecimal (`0x0000_0001`) literals with
/// underscore separators.
fn parse_u32_field(block: &str, field: &str) -> u32 {
    for line in block.lines() {
        let line = line.trim();
        // Match lines like `field_name: value,`
        if let Some(rest) = line.strip_prefix(field) {
            if let Some(rest) = rest.trim_start().strip_prefix(':') {
                let val = rest.trim().trim_end_matches(',').replace('_', "");
                if let Some(hex) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
                    return u32::from_str_radix(hex, 16).unwrap_or(0);
                }
                return val.parse::<u32>().unwrap_or(0);
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Host-side CCG construction and MIP computation
// ---------------------------------------------------------------------------

/// Build the CCG adjacency matrix: `edges[l][h]` is true when task l holds a
/// cap that task h requires.
fn build_edges(tasks: &[TaskConfig]) -> Vec<Vec<bool>> {
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

/// Compute MIP for every task via BFS over the CCG.  Returns a Vec where
/// `mip[i]` = MaxInheritedPriority for registry slot i (seeded with own priority).
fn compute_mip(tasks: &[TaskConfig]) -> Vec<u8> {
    let n = tasks.len();
    let edges = build_edges(tasks);
    let mut mip: Vec<u8> = tasks.iter().map(|t| t.priority).collect();

    for src in 0..n {
        let mut visited = vec![false; n];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(src);
        visited[src] = true;
        while let Some(cur) = queue.pop_front() {
            if tasks[cur].priority > mip[src] {
                mip[src] = tasks[cur].priority;
            }
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

/// DFS cycle detection; emit `cargo:warning` for each back-edge found.
fn check_cycles_and_warn(tasks: &[TaskConfig]) {
    let n = tasks.len();
    let edges = build_edges(tasks);
    let mut visited   = vec![false; n];
    let mut rec_stack = vec![false; n];

    for start in 0..n {
        if !visited[start] {
            dfs_cycle(start, &edges, &mut visited, &mut rec_stack);
        }
    }
}

fn dfs_cycle(u: usize, edges: &[Vec<bool>], visited: &mut Vec<bool>, rec: &mut Vec<bool>) {
    visited[u] = true;
    rec[u] = true;
    for v in 0..edges[u].len() {
        if edges[u][v] {
            if !visited[v] {
                dfs_cycle(v, edges, visited, rec);
            } else if rec[v] {
                println!(
                    "cargo:warning=CCG cycle detected: task {} → task {} → ... \
                     Mutual block — check capability declarations in tasks/demo_tasks.oml.",
                    u, v
                );
            }
        }
    }
    rec[u] = false;
}

// ---------------------------------------------------------------------------
// Emit ccg_constants.rs
// ---------------------------------------------------------------------------

/// Write `src/generated/ccg_constants.rs` containing a `[u8; MAX_TASKS]` array
/// of precomputed MIP values indexed by task registry slot.
fn emit_ccg_constants(out_dir: &Path, mip: &[u8]) {
    let mut array = [0u8; MAX_TASKS];
    for (i, &m) in mip.iter().enumerate().take(MAX_TASKS) {
        array[i] = m;
    }

    let entries: Vec<String> = array.iter().map(|v| v.to_string()).collect();
    let content = format!(
        "// Auto-generated by kernel/build.rs — do not edit manually.\n\
         // Rebuild after editing tasks/demo_tasks.oml.\n\
         //\n\
         // MAX_INHERITED_PRIORITIES[i] = MaxInheritedPriority for task registry\n\
         // slot i, computed via BFS over the Capability Contention Graph at\n\
         // build time.  The scheduler writes these values into TaskDescriptor\n\
         // at boot instead of running the BFS at runtime.\n\
         //\n\
         // Slot order matches task registration order in kernel_main:\n\
         //   slot 0 = TASK_L, slot 1 = TASK_M, slot 2 = TASK_H\n\
         pub const MAX_INHERITED_PRIORITIES: [u8; {max_tasks}] = [{entries}];\n",
        max_tasks = MAX_TASKS,
        entries = entries.join(", "),
    );

    std::fs::write(out_dir.join("ccg_constants.rs"), content)
        .expect("failed to write ccg_constants.rs");
}

// ---------------------------------------------------------------------------
// OML binary resolution
// ---------------------------------------------------------------------------

/// Locate the OML binary using the resolution order in the module doc.
fn find_oml_binary(manifest_dir: &Path) -> Option<String> {
    if let Ok(bin) = env::var("OML_BIN") {
        if Path::new(&bin).exists() {
            return Some(bin);
        }
        println!("cargo:warning=OML_BIN={} not found on disk.", bin);
    }

    // Git submodule at the repo root (oml/ relative to kernel/).
    let submodule = manifest_dir.join("../oml/target/release/oml");
    if submodule.exists() {
        return Some(submodule.to_string_lossy().into_owned());
    }

    // Sibling repository layout used during local development.
    let sibling = manifest_dir.join("../../OML/target/release/oml");
    if sibling.exists() {
        return Some(sibling.to_string_lossy().into_owned());
    }

    if Command::new("oml").arg("--help").output().is_ok() {
        return Some("oml".to_string());
    }

    None
}
