// kernel/build.rs — Cargo build script for the ferret kernel crate.
//
// Responsibilities:
//   1. Linker script setup — add kernel/ to the linker search path so riscv-rt
//      can find memory.x (which defines MEMORY regions for our target).
//   2. OML code generation — invoke the OML transpiler on tasks/*.oml to
//      regenerate src/generated/{task_schema,demo_tasks,pipeline_tasks}.rs.  The build continues without error if OML
//      is not built; the committed generated files are used instead.
//   3. CCG constant emission — parse each task set in src/generated/, compute the
//      Capability Contention Graph and MaxInheritedPriority on the host (with
//      alloc, via build/ccg_gen.rs), and write src/generated/ccg_constants.rs.
//      This eliminates all boot-time graph traversal from the kernel binary.
//
// OML binary resolution order:
//   1. OML_BIN env var (set by CI or developer)
//   2. oml/target/release/oml (git submodule at repo root; scripts/build_oml.sh)
//
// A sibling checkout or an `oml` on PATH is deliberately not used: an older
// transpiler without instance support silently emits empty instance files,
// which would overwrite the committed demo_tasks.rs.  The submodule pins the
// exact OML revision this kernel is generated with.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "build/ccg_gen.rs"]
mod ccg_gen;

const MAX_TASKS: usize = 16;

fn main() {
    // Declare `kani` as a known cfg name so the `unexpected_cfgs` lint does
    // not fire on `#[cfg(kani)]` / `#[cfg(not(kani))]` attributes.
    // Kani sets this flag when it compiles the crate for model checking.
    println!("cargo::rustc-check-cfg=cfg(kani)");

    // --- Linker script setup ------------------------------------------------
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-link-search={}", manifest_dir.display());
    println!("cargo:rerun-if-changed=memory.x");

    // --- OML code generation ------------------------------------------------
    // Each .oml source maps to the file name the kernel imports.  The OML
    // transpiler names its output after the input file stem, so it writes
    // into OUT_DIR and the result is copied to its final name; files are only
    // rewritten when their contents change, so an unchanged regeneration does
    // not dirty the working tree or retrigger dependent builds.
    println!("cargo:rerun-if-changed=../tasks/");
    println!("cargo:rerun-if-env-changed=OML_BIN");

    let out_dir = manifest_dir.join("src/generated");
    std::fs::create_dir_all(&out_dir).expect("failed to create src/generated/");

    const OML_SOURCES: [(&str, &str); 3] = [
        ("task.oml", "task_schema.rs"),
        ("demo_tasks.oml", "demo_tasks.rs"),
        ("pipeline_tasks.oml", "pipeline_tasks.rs"),
    ];

    if let Some(oml_bin) = find_oml_binary(&manifest_dir) {
        let tasks_dir = manifest_dir.join("../tasks");
        let scratch = PathBuf::from(env::var("OUT_DIR").unwrap()).join("oml");
        std::fs::create_dir_all(&scratch).expect("failed to create OUT_DIR/oml");
        for (source, target) in OML_SOURCES {
            let input = tasks_dir.join(source);
            let status = Command::new(&oml_bin)
                .arg(&input)
                .args(["--rust", "--output"])
                .arg(&scratch)
                .status()
                .expect("failed to invoke OML transpiler");
            assert!(status.success(), "OML transpiler failed on {}", source);

            let stem = Path::new(source).file_stem().unwrap().to_string_lossy().into_owned();
            let produced = std::fs::read_to_string(scratch.join(format!("{stem}.rs")))
                .unwrap_or_else(|_| panic!("OML produced no output for {}", source));
            write_if_changed(&out_dir.join(target), &produced);
        }
    } else {
        println!("cargo:warning=OML binary not found — using committed generated files.");
    }

    // --- CCG constant emission (closes #42) ---------------------------------
    // Always regenerate ccg_constants.rs from the committed task sets so the
    // kernel never has to run CCG construction or MIP BFS at boot.  The
    // algorithm lives in build/ccg_gen.rs, which the kernel's host unit tests
    // also compile, so the generator is tested like any other kernel code.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/ccg_gen.rs");

    let mut mips: Vec<Vec<u8>> = Vec::new();
    for (source, _) in ccg_gen::TASK_SETS {
        let path = out_dir.join(source);
        println!("cargo:rerun-if-changed=src/generated/{source}");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("failed to read src/generated/{source}"));
        let tasks = ccg_gen::parse_task_configs(&src);
        for (from, to) in ccg_gen::find_cycle_back_edges(&tasks) {
            println!(
                "cargo:warning=CCG cycle detected in {}: task {} → task {} → ... \
                 Mutual block — check capability declarations in tasks/*.oml.",
                source, from, to
            );
        }
        mips.push(ccg_gen::compute_mip(&tasks));
    }
    let tables: Vec<ccg_gen::MipTable<'_>> = ccg_gen::TASK_SETS
        .iter()
        .zip(&mips)
        .map(|(&(source, const_name), mip)| ccg_gen::MipTable { const_name, source, mip })
        .collect();
    write_if_changed(
        &out_dir.join("ccg_constants.rs"),
        &ccg_gen::render_ccg_constants(&tables, MAX_TASKS),
    );
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
    println!("cargo:rerun-if-changed={}", submodule.display());
    if submodule.exists() {
        return Some(submodule.to_string_lossy().into_owned());
    }

    None
}

/// Write `contents` to `path` unless the file already holds exactly that.
fn write_if_changed(path: &Path, contents: &str) {
    if std::fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return;
    }
    std::fs::write(path, contents)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
}
