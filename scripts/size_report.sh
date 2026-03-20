#!/usr/bin/env bash
# scripts/size_report.sh — Report kernel binary size vs memory budgets.
#
# Why does this exist?
# On constrained embedded targets, running out of flash or RAM is a hard
# failure — the linker will catch it, but only at link time.  This script lets
# us track size trends *before* they become link errors, and it fails CI when
# we exceed budget so size regressions are caught immediately.
#
# Budget (from the linker script):
#   FLASH: 512 KB  — .text + .rodata must fit here
#   RAM  : 256 KB  — .data + .bss must fit here
#
# Usage:
#   ./scripts/size_report.sh          # uses release build (build first)
#   ./scripts/size_report.sh --debug  # uses debug build

set -euo pipefail

FLASH_BUDGET=$((512 * 1024))   # 524288 bytes
RAM_BUDGET=$((256 * 1024))     # 262144 bytes

PROFILE="release"
for arg in "$@"; do
    case "$arg" in
        --debug) PROFILE="debug" ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL_ELF="$REPO_ROOT/target/riscv32imac-unknown-none-elf/$PROFILE/ferret"

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: $KERNEL_ELF not found. Run 'cargo build --release' first." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Find a suitable `size` tool
# ---------------------------------------------------------------------------
# We prefer llvm-size because it understands ELF sections by name and is
# shipped with the Rust toolchain (via the llvm-tools component).  We fall
# back to the system `size` if llvm-size is not available.
SIZE_CMD=""

# llvm-size may be installed as `rust-size` (via cargo-binutils) or as a
# versioned binary like `llvm-size-17`.  Try common names in order.
for candidate in rust-size llvm-size llvm-size-17 llvm-size-16 size; do
    if command -v "$candidate" &>/dev/null; then
        SIZE_CMD="$candidate"
        break
    fi
done

if [[ -z "$SIZE_CMD" ]]; then
    echo "WARNING: No suitable 'size' tool found." >&2
    echo "  Install cargo-binutils ('cargo install cargo-binutils') or" >&2
    echo "  the llvm-tools rustup component ('rustup component add llvm-tools')." >&2
    echo "  Skipping size check — budget enforcement disabled." >&2
    exit 0
fi

# ---------------------------------------------------------------------------
# Parse section sizes
# ---------------------------------------------------------------------------
# `size --format=sysv` prints one line per section: Name  Size  Address
# We extract .text, .rodata, .data, .bss.

RAW_OUTPUT=$("$SIZE_CMD" --format=sysv "$KERNEL_ELF" 2>/dev/null || \
             "$SIZE_CMD" "$KERNEL_ELF" 2>/dev/null)

# Helper: extract a named section's size from sysv output.
get_section_size() {
    local section_name="$1"
    # The sysv format has lines like:  .text            12345    2147483648
    # We match on the section name and grab the second column (decimal size).
    echo "$RAW_OUTPUT" | awk -v sec="$section_name" '
        $1 == sec { print $2; found=1 }
        END { if (!found) print 0 }
    '
}

TEXT_SIZE=$(get_section_size ".text")
RODATA_SIZE=$(get_section_size ".rodata")
DATA_SIZE=$(get_section_size ".data")
BSS_SIZE=$(get_section_size ".bss")

# If sysv mode didn't work (some older size tools default to BSD mode),
# try to parse the simpler BSD output: text   data    bss     dec     hex
if [[ "$TEXT_SIZE" -eq 0 && "$RODATA_SIZE" -eq 0 && "$DATA_SIZE" -eq 0 && "$BSS_SIZE" -eq 0 ]]; then
    BSD_LINE=$(echo "$RAW_OUTPUT" | grep -v "^text" | grep -v "^$" | head -1)
    TEXT_SIZE=$(echo "$BSD_LINE" | awk '{print $1}')
    DATA_SIZE=$(echo "$BSD_LINE" | awk '{print $2}')
    BSS_SIZE=$(echo "$BSD_LINE" | awk '{print $3}')
    RODATA_SIZE=0  # BSD format folds rodata into text
fi

FLASH_USED=$(( TEXT_SIZE + RODATA_SIZE ))
RAM_USED=$(( DATA_SIZE + BSS_SIZE ))

# ---------------------------------------------------------------------------
# Print the report
# ---------------------------------------------------------------------------
human_kb() {
    echo "$(( ($1 + 512) / 1024 )) KB ($1 B)"
}

echo "=========================================="
echo "  FerretOS kernel size report"
echo "  Profile : $PROFILE"
echo "  Tool    : $SIZE_CMD"
echo "=========================================="
printf "  %-10s %8s bytes\n" ".text"   "$TEXT_SIZE"
printf "  %-10s %8s bytes\n" ".rodata" "$RODATA_SIZE"
printf "  %-10s %8s bytes\n" ".data"   "$DATA_SIZE"
printf "  %-10s %8s bytes\n" ".bss"    "$BSS_SIZE"
echo "------------------------------------------"
printf "  %-10s %8s / %s\n"  "FLASH"   "$(human_kb $FLASH_USED)" "$(human_kb $FLASH_BUDGET)"
printf "  %-10s %8s / %s\n"  "RAM"     "$(human_kb $RAM_USED)"   "$(human_kb $RAM_BUDGET)"
echo "=========================================="

# ---------------------------------------------------------------------------
# Budget enforcement
# ---------------------------------------------------------------------------
FAIL=false

if [[ "$FLASH_USED" -gt "$FLASH_BUDGET" ]]; then
    echo "FAIL: FLASH budget exceeded by $(( FLASH_USED - FLASH_BUDGET )) bytes!" >&2
    FAIL=true
else
    echo "  FLASH: OK ($(( FLASH_BUDGET - FLASH_USED )) bytes remaining)"
fi

if [[ "$RAM_USED" -gt "$RAM_BUDGET" ]]; then
    echo "FAIL: RAM budget exceeded by $(( RAM_USED - RAM_BUDGET )) bytes!" >&2
    FAIL=true
else
    echo "  RAM  : OK ($(( RAM_BUDGET - RAM_USED )) bytes remaining)"
fi

echo "=========================================="

if $FAIL; then
    exit 1
fi
