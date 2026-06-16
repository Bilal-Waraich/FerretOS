// kernel/build.rs — Cargo build script for the ferret kernel crate.
//
// Responsibilities:
//   1. Linker script setup — add kernel/ to the linker search path so riscv-rt
//      can find memory.x (which defines MEMORY regions for our target).
//   2. OML code generation — invoke the OML transpiler on tasks/*.oml to
//      regenerate kernel/src/generated/task_schema.rs.  The build continues
//      without error if OML is not installed; the committed generated files
//      are used instead.
//
// OML binary resolution order:
//   1. OML_BIN env var (set by CI or developer)
//   2. ../../OML/target/release/oml (sibling repo, local development layout)
//   3. oml on PATH

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // --- Linker script setup ------------------------------------------------
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-link-search={}", manifest_dir.display());
    println!("cargo:rerun-if-changed=memory.x");

    // --- OML code generation ------------------------------------------------
    println!("cargo:rerun-if-changed=../tasks/");
    println!("cargo:rerun-if-env-changed=OML_BIN");

    let oml_bin = match find_oml_binary(&manifest_dir) {
        Some(p) => p,
        None => {
            println!("cargo:warning=OML binary not found — using committed generated files.");
            return;
        }
    };

    let tasks_dir = manifest_dir.join("../tasks");
    let out_dir   = manifest_dir.join("src/generated");

    if !tasks_dir.exists() {
        println!("cargo:warning=tasks/ directory not found — skipping OML generation.");
        return;
    }

    std::fs::create_dir_all(&out_dir).expect("failed to create src/generated/");

    // Generate Rust types from the base schema only.  Instance generation
    // (demo_tasks.oml) requires planned OML extensions and is handled by the
    // hand-written kernel/src/generated/tasks.rs until those land.
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
