#!/usr/bin/env bash
# scripts/build_oml.sh — build the pinned OML transpiler from the submodule.
#
# Once oml/target/release/oml exists, `cargo build` regenerates
# kernel/src/generated/{task_schema,demo_tasks}.rs from tasks/*.oml.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if [[ ! -f oml/Cargo.toml ]]; then
    git submodule update --init oml
fi

# OML builds on stable Rust for the host; the repo's pinned nightly and
# riscv32 default target do not apply to it.
cargo +stable build --release --manifest-path oml/Cargo.toml --target-dir oml/target \
    --config 'build.target="'"$(rustc +stable -vV | sed -n 's/^host: //p')"'"'
# --config build.target puts the binary in a triple-specific directory; link
# it to the path build.rs expects.
HOST="$(rustc +stable -vV | sed -n 's/^host: //p')"
mkdir -p oml/target/release
cp "oml/target/$HOST/release/oml" oml/target/release/oml
echo "OML built: oml/target/release/oml"
