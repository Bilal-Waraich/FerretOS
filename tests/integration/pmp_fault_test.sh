#!/usr/bin/env bash
# tests/integration/pmp_fault_test.sh — PMP violation integration test (#89).
#
# Builds the kernel with the `fault-demo` task, which stores to a kernel-owned
# word outside its memory region, and checks both sides of the guarantee:
#
#   1. features pmp,fault-demo — the store must trap: the kernel reports a
#      store access fault from U-mode with mtval equal to the target address,
#      names the rogue task, and halts.
#   2. feature fault-demo only — with no isolation the same store lands and
#      the task says so.  This shows the PMP layer is what stops it.
#
# Leaves a default release build in place when it finishes.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KERNEL_ELF="$REPO_ROOT/target/riscv32imac-unknown-none-elf/release/ferret"
QEMU="qemu-system-riscv32"
TIMEOUT_SEC=3
PASS=0
FAIL=0
cd "$REPO_ROOT"

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
check() {
    local label="$1"; shift
    if "$@"; then green "  PASS  $label"; PASS=$((PASS + 1)); else red "  FAIL  $label"; FAIL=$((FAIL + 1)); fi
}

boot() { # boot <features> <log>
    cargo build --release --features "$1" >/dev/null 2>&1
    "$QEMU" -machine virt -nographic -bios none -kernel "$KERNEL_ELF" > "$2" 2>&1 &
    local qpid=$!
    sleep "$TIMEOUT_SEC"
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null || true
}

LOG_PMP="$(mktemp)"
LOG_NOPMP="$(mktemp)"
trap 'rm -f "$LOG_PMP" "$LOG_NOPMP"' EXIT

echo "Running PMP violation test..."
boot "pmp,fault-demo" "$LOG_PMP"
boot "fault-demo" "$LOG_NOPMP"
cargo build --release >/dev/null 2>&1

TARGET=$(sed -n 's/^\[F\] writing to kernel memory at \(0x[0-9a-f]*\).*/\1/p' "$LOG_PMP" | head -1)
MTVAL=$(sed -n 's/^\[FAULT\] .* mtval=\(0x[0-9a-f]*\) .*/\1/p' "$LOG_PMP" | head -1)

echo ""
echo "With PMP:"
grep -E '^\[F\]|^\[FAULT\]|halted' "$LOG_PMP" | sed 's/^/    /'
check "rogue task attempted the store"        grep -q '^\[F\] writing to kernel memory' "$LOG_PMP"
check "store access fault from U-mode"        grep -q '^\[FAULT\] store access fault (PMP violation).* mode=U' "$LOG_PMP"
check "fault names the rogue task (task=3)"   grep -q '^\[FAULT\] .* task=3 ' "$LOG_PMP"
check "mtval equals target address ($TARGET)" test -n "$TARGET" -a "$TARGET" = "$MTVAL"
check "store did not succeed"                 bash -c "! grep -q 'SUCCEEDED' '$LOG_PMP'"
check "kernel halted"                         grep -q 'System halted' "$LOG_PMP"

echo ""
echo "Without PMP:"
grep -E '^\[F\]|^\[FAULT\]' "$LOG_NOPMP" | sed 's/^/    /'
check "store lands without isolation"         grep -q 'write to kernel memory SUCCEEDED' "$LOG_NOPMP"
check "no fault raised"                       bash -c "! grep -q '^\[FAULT\]' '$LOG_NOPMP'"

echo ""
echo "Results: $PASS passed, $FAIL failed."
if [[ $FAIL -gt 0 ]]; then red "PMP fault test FAILED."; exit 1; fi
green "PMP fault test PASSED."
