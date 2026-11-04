#!/usr/bin/env bash
# scripts/host_test.sh — run the kernel library's unit tests on the host.
#
# .cargo/config.toml defaults every build to riscv32imac, where the Rust test
# harness cannot run.  This script overrides the target with the host triple.
# Extra arguments are forwarded to `cargo test` (e.g. `--features pmp`).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST="$(rustc -vV | sed -n 's/^host: //p')"

cd "$REPO_ROOT"
cargo test -p ferret --lib --target "$HOST" "$@"
cargo test -p ferret --lib --target "$HOST" --features pmp "$@"
