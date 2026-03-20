// kernel/build.rs — Cargo build script for the ferret kernel crate.
//
// What does this do?
// riscv-rt's link.x contains the line `INCLUDE memory.x`, which tells the
// linker to look for a file called `memory.x` in its search path.  By default
// that search path only includes riscv-rt's own output directory.  We use
// this build script to add *our* kernel directory to the search path so the
// linker finds our memory.x (which defines the MEMORY regions for our target).
//
// Without this, riscv-rt would fail to link because memory.x would not be found.

use std::env;
use std::path::PathBuf;

fn main() {
    // CARGO_MANIFEST_DIR is the path to the directory containing this Cargo.toml.
    // For us that is `kernel/`, where memory.x also lives.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Tell the linker to search this directory for linker scripts (memory.x).
    println!("cargo:rustc-link-search={}", manifest_dir.display());

    // Rerun this build script if memory.x changes so the linker picks up the
    // new addresses without needing a `cargo clean`.
    println!("cargo:rerun-if-changed=memory.x");
}
