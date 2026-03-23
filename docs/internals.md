# FerretOS — Kernel Internals

This document tracks the implementation state of the kernel, module by module.
It is updated in step with code changes and serves as the ground-truth reference
for what exists, how it works, and why key decisions were made.

---

## Sprint 0 — Foundations

### Workspace (`Cargo.toml`, `rust-toolchain.toml`)

- Single Cargo workspace rooted at the repo root.
- One member crate: `kernel/` (binary `ferret`).
- Rust toolchain pinned to `nightly-2024-12-01` via `rust-toolchain.toml`.
- Release profile: `opt-level = "z"`, `lto = true`, `codegen-units = 1`.
- Target: `riscv32imac-unknown-none-elf`, configured in `.cargo/config.toml`.

### Linker script (`linker/ferret.ld`)

Memory map on QEMU `virt` (riscv32):

| Region | Base         | Length | Flags |
|--------|--------------|--------|-------|
| FLASH  | `0x8000_0000`| 512 KB | r-x   |
| RAM    | `0x8008_0000`| 256 KB | rwx   |

Sections in order: `.text` → `.rodata` → `.data` (init image in FLASH, VMA in RAM) → `.bss` (zero-init in RAM). Stack grows down from the top of RAM. Link-time `ASSERT`s guard both budgets.

### Boot entry (`kernel/src/main.rs`)

- `#![no_std]`, `#![no_main]`; entry via `riscv_rt::entry` macro (`_start_rust`).
- riscv-rt's `boot.S` runs first: sets `sp`, zeroes BSS, copies `.data`.
- `kernel_main` prints a boot banner and memory-map diagnostics over UART, then spins in `wfi`.

### UART driver (`kernel/src/uart.rs`)

16550-compatible UART at `0x1000_0000` (QEMU virt device-tree hardcoded address).

| Function         | Description                                    |
|------------------|------------------------------------------------|
| `uart_putchar(u8)` | Single volatile byte write to THR (offset 0) |
| `uart_puts(&str)` | Iterates bytes, calls `uart_putchar`          |
| `uart_print_hex(usize)` | `0x`-prefixed hex, MSN first           |
| `uart_print_usize(usize)` | Decimal, no leading zeros             |

All MMIO writes use `write_volatile` to prevent the compiler from eliding them.

### Panic handler (`kernel/src/panic.rs`)

- `#[panic_handler]` prints `!!! KERNEL PANIC !!!`, the message, and source location to UART via `UartWriter` (a `fmt::Write` adapter over `uart_puts`).
- Spins forever with `core::hint::spin_loop()` after printing.

### CI / scripts

| Script / Workflow | Purpose |
|---|---|
| `scripts/run_qemu.sh` | Launches QEMU; `--gdb` adds `-s -S`; `--debug` uses debug build |
| `scripts/size_report.sh` | Asserts `.text+.rodata ≤ 512 KB`, `.data+.bss ≤ 256 KB` |
| `scripts/gdb_attach.sh` | Connects `riscv32-unknown-elf-gdb` to QEMU GDB stub |
| `scripts/boot_test.sh` | Boots QEMU, asserts "Ferret booting..." appears in output |
| `.github/workflows/build.yml` | `cargo build --release` + boot test + size report |
| `.github/workflows/clippy.yml` | `cargo clippy -- -D warnings` (zero-warning policy) |

---

## Sprint 1 — Kernel Core

### TrapFrame (`kernel/src/context/mod.rs`) — Issue #12

`TrapFrame` is the in-memory representation of the full RISC-V machine state at
the moment a trap (exception or interrupt) is taken.  The assembly trap stub
saves register state into this struct before calling the Rust `trap_handler`,
and restores it on the way back out.

```
#[repr(C)]
pub struct TrapFrame {
    regs:     [usize; 32],  // x0–x31 (x0 always zero, kept for index uniformity)
    mepc:     usize,        // address to mret to
    mstatus:  usize,        // includes MPP for privilege-level return (Sprint 3)
    mcause:   usize,        // bit 31 = interrupt; bits 30:0 = cause code
    mtval:    usize,        // faulting address for address exceptions; 0 otherwise
}
```

`#[repr(C)]` is non-negotiable: the assembly stub accesses fields by hardcoded
byte offsets (`0×00` … `0×90`).  Total size: 36 × 4 = 144 bytes.

All 32 GPRs are saved regardless of calling convention so the interrupted
context can be resumed transparently.  `mstatus` is saved in full so that
`mret` correctly restores the privilege level when user-mode tasks are added.
