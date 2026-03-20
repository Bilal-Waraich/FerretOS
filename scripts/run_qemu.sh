#!/usr/bin/env bash
# scripts/run_qemu.sh — Build FerretOS and run it under QEMU.
#
# Usage:
#   ./scripts/run_qemu.sh            # release build, run normally
#   ./scripts/run_qemu.sh --debug    # debug build (unoptimised)
#   ./scripts/run_qemu.sh --gdb      # release build, pause at reset and wait
#                                    # for a GDB connection on localhost:1234
#   ./scripts/run_qemu.sh --debug --gdb   # both flags together

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
PROFILE="release"
GDB_MODE=false

for arg in "$@"; do
    case "$arg" in
        --debug)
            PROFILE="debug"
            ;;
        --gdb)
            GDB_MODE=true
            ;;
        --help|-h)
            grep '^#' "$0" | head -20 | sed 's/^# \?//'
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            echo "Run '$0 --help' for usage." >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Sanity checks
# ---------------------------------------------------------------------------

# Make sure QEMU is actually installed before spending time building.
if ! command -v qemu-system-riscv32 &>/dev/null; then
    echo "ERROR: qemu-system-riscv32 not found in PATH." >&2
    echo "" >&2
    echo "Install it with one of:" >&2
    echo "  macOS  : brew install qemu" >&2
    echo "  Debian : sudo apt-get install qemu-system-misc" >&2
    echo "  Fedora : sudo dnf install qemu-system-riscv" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Building FerretOS (profile: $PROFILE)…"
if [[ "$PROFILE" == "release" ]]; then
    cargo build --release
    KERNEL_ELF="target/riscv32imac-unknown-none-elf/release/ferret"
else
    cargo build
    KERNEL_ELF="target/riscv32imac-unknown-none-elf/debug/ferret"
fi

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: Expected ELF not found at $KERNEL_ELF" >&2
    exit 1
fi

echo "==> Kernel ELF: $KERNEL_ELF"
echo ""

# ---------------------------------------------------------------------------
# Assemble the QEMU command line
# ---------------------------------------------------------------------------
QEMU_ARGS=(
    -machine virt       # Generic RISC-V virtual board — matches our memory map
    -nographic          # No GUI; serial output goes to the terminal
    -bios none          # Skip OpenSBI; we own the machine from address 0
    -kernel "$KERNEL_ELF"
)

if $GDB_MODE; then
    # -s        opens a GDB stub on tcp::1234
    # -S        freezes the CPU at reset so GDB can set breakpoints before code runs
    QEMU_ARGS+=(-s -S)
    echo "==> GDB mode: QEMU paused at reset."
    echo "    In another terminal run:"
    echo ""
    echo "      riscv32-unknown-elf-gdb $KERNEL_ELF \\"
    echo "        -ex 'target remote :1234' \\"
    echo "        -ex 'break kernel_main' \\"
    echo "        -ex 'continue'"
    echo ""
    echo "    Or use scripts/gdb_attach.sh for a pre-configured session."
    echo ""
fi

echo "==> Starting QEMU…"
echo "    (Press Ctrl-A X to exit QEMU)"
echo ""

exec qemu-system-riscv32 "${QEMU_ARGS[@]}"
