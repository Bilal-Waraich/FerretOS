#!/usr/bin/env bash
# scripts/gdb_attach.sh — Start a GDB session attached to a QEMU stub.
#
# Usage:
#   1. In one terminal:  ./scripts/run_qemu.sh --gdb
#   2. In another terminal: ./scripts/gdb_attach.sh
#
# GDB will connect to QEMU's remote-debugging stub on localhost:1234,
# set a breakpoint at `kernel_main`, and run to it.  From there you can
# step through Rust code, inspect registers, and examine memory.
#
# Required: a RISC-V capable GDB.  Possible binary names:
#   - riscv32-unknown-elf-gdb      (from a GNU toolchain)
#   - riscv64-unknown-elf-gdb      (also handles 32-bit in most builds)
#   - gdb-multiarch                (Debian/Ubuntu generic multi-arch GDB)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL_ELF="$REPO_ROOT/target/riscv32imac-unknown-none-elf/release/ferret"

if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: $KERNEL_ELF not found.  Build with: cargo build --release" >&2
    exit 1
fi

# Find a suitable GDB binary
GDB_CMD=""
for candidate in riscv32-unknown-elf-gdb riscv64-unknown-elf-gdb gdb-multiarch gdb; do
    if command -v "$candidate" &>/dev/null; then
        GDB_CMD="$candidate"
        break
    fi
done

if [[ -z "$GDB_CMD" ]]; then
    echo "ERROR: No RISC-V-capable GDB found in PATH." >&2
    echo "  macOS:   brew install riscv-software-src/riscv/riscv-gnu-toolchain" >&2
    echo "  Debian:  sudo apt-get install gdb-multiarch" >&2
    exit 1
fi

echo "==> Connecting to QEMU GDB stub on localhost:1234 with $GDB_CMD"
echo "    Make sure 'run_qemu.sh --gdb' is running in another terminal."
echo ""

exec "$GDB_CMD" "$KERNEL_ELF" \
    -ex "set architecture riscv:rv32" \
    -ex "target remote :1234" \
    -ex "break kernel_main" \
    -ex "continue"
