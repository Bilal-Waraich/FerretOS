#!/usr/bin/env bash
# tests/integration/boot_test.sh — QEMU boot integration test.
#
# This is the primary test mechanism for FerretOS.  Because the kernel is
# bare-metal no_std, Rust's built-in test harness (cargo test) cannot run
# on the target: there is no `test` crate, no allocator, and no way to
# report results through a standard OS interface.
#
# Instead we run the kernel in QEMU, capture its serial output, and assert
# that expected strings appear.  This gives us end-to-end confidence that:
#   1. The toolchain produced a valid ELF.
#   2. QEMU boots it without trapping.
#   3. The UART driver works.
#   4. The kernel reaches our #[entry] function.
#
# Called by CI (build.yml) and can be run locally with:
#   bash tests/integration/boot_test.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KERNEL_ELF="$REPO_ROOT/target/riscv32imac-unknown-none-elf/release/ferret"
QEMU="qemu-system-riscv32"
TIMEOUT_SEC=5
PASS=0
FAIL=0

# ── Helpers ──────────────────────────────────────────────────────────────────

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }

assert_contains() {
    local label="$1" pattern="$2" output="$3"
    if echo "$output" | grep -qF "$pattern"; then
        green "  PASS  $label"
        PASS=$((PASS + 1))
    else
        red   "  FAIL  $label"
        red   "        expected to find: '$pattern'"
        FAIL=$((FAIL + 1))
    fi
}

# ── Preflight ─────────────────────────────────────────────────────────────────

if ! command -v "$QEMU" &>/dev/null; then
    echo "ERROR: $QEMU not found.  Install with: brew install qemu  /  apt-get install qemu-system-misc" >&2
    exit 1
fi

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: kernel ELF not found at $KERNEL_ELF" >&2
    echo "       Run 'cargo build --release' first." >&2
    exit 1
fi

# ── Boot capture ──────────────────────────────────────────────────────────────

echo "Running QEMU boot test (${TIMEOUT_SEC}s timeout)..."

# We use a background process + sleep rather than the `timeout` command because
# BSD/macOS and GNU coreutils spell it differently and have different defaults.
"$QEMU" -machine virt -nographic -bios none -kernel "$KERNEL_ELF" \
    > /tmp/ferret_boot_test.log 2>&1 &
QPID=$!
sleep "$TIMEOUT_SEC"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null || true

BOOT_LOG=$(cat /tmp/ferret_boot_test.log)

echo ""
echo "─── Serial output ────────────────────────────────────────────────────────"
echo "$BOOT_LOG"
echo "──────────────────────────────────────────────────────────────────────────"
echo ""

# ── Assertions ────────────────────────────────────────────────────────────────

echo "Assertions:"
assert_contains "boot banner"        "Ferret booting"       "$BOOT_LOG"
assert_contains "kernel name+ver"    "FerretOS v"           "$BOOT_LOG"
assert_contains "target line"        "riscv32imac"          "$BOOT_LOG"
assert_contains "memory map printed" "Memory map"           "$BOOT_LOG"
assert_contains "text start"         ".text start"          "$BOOT_LOG"
assert_contains "stack top"          "stack top"            "$BOOT_LOG"
assert_contains "idle message"       "Kernel idle"          "$BOOT_LOG"

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "Results: $PASS passed, $FAIL failed."

if [[ $FAIL -gt 0 ]]; then
    red "Boot test FAILED."
    exit 1
else
    green "Boot test PASSED."
fi
