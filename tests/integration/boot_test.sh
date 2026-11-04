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
#   5. The scheduler really runs tasks: all three demo tasks produce output,
#      timer-driven context switches happen, and the CA-PIP ordering property
#      holds (no M or H run starts strictly inside an L work window).
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
    local label="$1" pattern="$2" file="$3"
    if grep -qF "$pattern" "$file"; then
        green "  PASS  $label"
        PASS=$((PASS + 1))
    else
        red   "  FAIL  $label"
        red   "        expected to find: '$pattern'"
        FAIL=$((FAIL + 1))
    fi
}

assert_absent() {
    local label="$1" pattern="$2" file="$3"
    if grep -qF "$pattern" "$file"; then
        red   "  FAIL  $label"
        red   "        unexpected: '$pattern'"
        FAIL=$((FAIL + 1))
    else
        green "  PASS  $label"
        PASS=$((PASS + 1))
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

LOG=/tmp/ferret_boot_test.log

echo ""
echo "─── Serial output ────────────────────────────────────────────────────────"
head -40 "$LOG"
echo "──────────────────────────────────────────────────────────────────────────"
echo ""

# ── Assertions ────────────────────────────────────────────────────────────────

echo "Assertions:"
assert_contains "boot banner"        "Ferret booting"       "$LOG"
assert_contains "kernel name+ver"    "FerretOS v"           "$LOG"
assert_contains "target line"        "riscv32imac"          "$LOG"
assert_contains "memory map printed" "Memory map"           "$LOG"
assert_contains "text start"         ".text start"          "$LOG"
assert_contains "stack top"          "stack top"            "$LOG"
assert_contains "tasks registered"   "Tasks registered: 4"  "$LOG"
assert_contains "channel check"      "Channel check passed" "$LOG"
assert_contains "interrupts enabled" "Interrupts enabled"   "$LOG"
assert_contains "task L in U-mode"   "task 0 running in U-mode" "$LOG"
assert_contains "task M in U-mode"   "task 1 running in U-mode" "$LOG"
assert_contains "task H in U-mode"   "task 2 running in U-mode" "$LOG"
assert_contains "task L runs"        "[L] work start"       "$LOG"
assert_contains "task L finishes"    "[L] work end"         "$LOG"
assert_contains "task M runs"        "[M] run"              "$LOG"
assert_contains "task H runs"        "[H] run"              "$LOG"
assert_contains "switch latency"     "Switch latency"       "$LOG"
assert_contains "log validation"     "log of kernel memory rejected (EFAULT)" "$LOG"
assert_absent   "no faults"          "[FAULT]"              "$LOG"
assert_absent   "no panics"          "KERNEL PANIC"         "$LOG"

# CA-PIP ordering: L holds UART0 and runs at effective priority 3, so no
# M (priority 2) or H (equal priority, work shorter than the quantum) run may
# start strictly between an L "work start" and the matching "work end".
VIOLATIONS=$(awk '
    /^\[L\] work start t=/ { split($0, a, "t="); split(a[2], b, " "); s = b[1] + 0; open = 1; next }
    /^\[L\] work end t=/   { split($0, a, "t="); e = a[2] + 0;
                              for (i = 0; i < n; i++) if (t[i] > s && t[i] < e) bad++;
                              n = 0; open = 0; next }
    /^\[[MH]\] run t=/      { if (open) { split($0, a, "t="); t[n++] = a[2] + 0 } }
    END { print bad + 0 }
' "$LOG")
WINDOWS=$(grep -c '^\[L\] work end' "$LOG" || true)
if [[ "$VIOLATIONS" -eq 0 && "$WINDOWS" -gt 0 ]]; then
    green "  PASS  CA-PIP ordering ($WINDOWS L windows, 0 violations)"
    PASS=$((PASS + 1))
else
    red   "  FAIL  CA-PIP ordering ($WINDOWS L windows, $VIOLATIONS violations)"
    FAIL=$((FAIL + 1))
fi

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "Results: $PASS passed, $FAIL failed."

if [[ $FAIL -gt 0 ]]; then
    red "Boot test FAILED."
    exit 1
else
    green "Boot test PASSED."
fi
