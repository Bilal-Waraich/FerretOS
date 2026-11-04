#!/usr/bin/env bash
# scripts/switch_latency.sh — compare context-switch latency with and without
# per-task PMP (#88).
#
# Boots the default and `pmp` builds under QEMU's -icount mode, collects the
# kernel's "Switch latency" statistics (retired instructions per timer-driven
# switch, including the trap stub) and prints a comparison.  Exits non-zero if
# either build's worst case exceeds the 1000-instruction budget.
#
# QEMU is not cycle-accurate: instruction counts are deterministic and a fair
# proxy for cycles on a simple in-order RV32 core, but they are not cycles.
# Leaves a default release build in place.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL_ELF="$REPO_ROOT/target/riscv32imac-unknown-none-elf/release/ferret"
BUDGET=1000
SECONDS_PER_RUN=5
cd "$REPO_ROOT"

measure() { # measure <features|-> -> prints "min max avg samples"
    if [[ "$1" == "-" ]]; then cargo build --release >/dev/null 2>&1
    else cargo build --release --features "$1" >/dev/null 2>&1; fi
    local log; log="$(mktemp)"
    qemu-system-riscv32 -machine virt -nographic -bios none -icount shift=0 \
        -kernel "$KERNEL_ELF" > "$log" 2>&1 &
    local qpid=$!
    sleep "$SECONDS_PER_RUN"
    kill "$qpid" 2>/dev/null; wait "$qpid" 2>/dev/null || true
    awk '/^Switch latency:/ {
            for (i = 1; i <= NF; i++) {
                split($i, kv, "=");
                if (kv[1] == "min" && (mn == "" || kv[2] < mn)) mn = kv[2] + 0;
                if (kv[1] == "max" && kv[2] + 0 > mx) mx = kv[2] + 0;
                if (kv[1] == "avg") { sum += kv[2]; n++ }
            }
         }
         END { if (n == 0) print "0 0 0 0"; else printf "%d %d %d %d\n", mn, mx, sum / n, n * 100 }' "$log"
    rm -f "$log"
}

read -r MIN_D MAX_D AVG_D N_D <<<"$(measure -)"
read -r MIN_P MAX_P AVG_P N_P <<<"$(measure pmp)"
cargo build --release >/dev/null 2>&1

echo "Context-switch latency (retired instructions, QEMU -icount, incl. 84-instruction trap stub)"
printf "  %-8s %8s %8s %8s %9s\n" "build" "min" "avg" "max" "samples"
printf "  %-8s %8s %8s %8s %9s\n" "default" "$MIN_D" "$AVG_D" "$MAX_D" "$N_D"
printf "  %-8s %8s %8s %8s %9s\n" "pmp" "$MIN_P" "$AVG_P" "$MAX_P" "$N_P"
printf "  PMP overhead (avg): %+d instructions\n" "$(( AVG_P - AVG_D ))"
echo "  Budget: $BUDGET instructions"

if [[ "$N_D" -eq 0 || "$N_P" -eq 0 ]]; then echo "ERROR: no latency samples collected" >&2; exit 1; fi
if [[ "$MAX_D" -gt "$BUDGET" || "$MAX_P" -gt "$BUDGET" ]]; then echo "FAIL: worst case exceeds budget" >&2; exit 1; fi
echo "OK: both builds within budget"
