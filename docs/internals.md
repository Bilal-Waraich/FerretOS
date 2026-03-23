# FerretOS — Kernel Internals

Ground-truth reference for what has been built, how each component works, and
why key implementation decisions were made.  Updated in lockstep with code.
Architecture rationale and algorithm proofs live in `FERRET.md`.

---

## Status

| Sprint | Theme | Issues | Status |
|--------|-------|--------|--------|
| 0 | Foundations — toolchain + QEMU boot | #1–#11 | ✅ Complete |
| 1 | Kernel core — interrupts + context switch | #12–#18 | ✅ Complete |
| 2 | Memory safety layer — regions + registry | #19–#25 | 🔲 Pending |
| 3 | Capability system — ZST types + boot conflict detection | #26–#33 | 🔲 Pending |
| 4 | Scheduler — CA-PIP + CCG | #34–#44 | 🔲 Pending |
| 5 | OML integration — transpiler + code generation | #45–#52 | 🔲 Pending |
| 6 | Polish and demo | #53–#61 | 🔲 Pending |

---

## Sprint 0 — Foundations

### Cargo workspace (`Cargo.toml`, `rust-toolchain.toml`) ✅

Single workspace, one binary crate: `kernel/` → binary `ferret`.
Toolchain pinned to `nightly-2024-12-01`.  Release profile: `opt-level = "z"`,
`lto = "fat"`, `codegen-units = 1`.  Target triple `riscv32imac-unknown-none-elf`
is set in `.cargo/config.toml`.

### Linker script (`linker/ferret.ld`) ✅

| Region | Base | Length | Flags |
|--------|------|--------|-------|
| FLASH | `0x8000_0000` | 512 KB | r-x |
| RAM | `0x8008_0000` | 256 KB | rwx |

Section order: `.text` → `.rodata` → `.data` (LMA in FLASH, VMA in RAM) →
`.bss` (zero-init).  Stack grows down from `ORIGIN(RAM) + LENGTH(RAM)`.
Link-time `ASSERT`s fail the build if either budget is exceeded.

### Boot entry (`kernel/src/main.rs`) ✅

`#![no_std]`, `#![no_main]`; entry via `riscv_rt::entry` (`_start_rust`).
riscv-rt `boot.S` runs first: sets `sp`, zeroes BSS, copies `.data`.
`kernel_main` prints a boot banner and memory-map diagnostics, then arms
the CLINT timer and enables interrupts (Sprint 1).

### UART driver (`kernel/src/uart.rs`) ✅

16550-compatible UART at `0x1000_0000` (QEMU virt device tree).

| Function | Description |
|---|---|
| `uart_putchar(u8)` | Single `write_volatile` to THR (offset 0) |
| `uart_puts(&str)` | Iterates bytes, calls `uart_putchar` |
| `uart_print_hex(usize)` | `0x`-prefixed, zero-padded hex, MSN first |
| `uart_print_usize(usize)` | Decimal, no leading zeros, 20-byte stack buffer |

All MMIO writes use `write_volatile` — the compiler cannot see UART side effects
and would otherwise elide repeated writes to the same address.

### Panic handler (`kernel/src/panic.rs`) ✅

`#[panic_handler]` prints `!!! KERNEL PANIC !!!`, the message, and source
location via `UartWriter` (a `fmt::Write` adapter over `uart_puts`), then spins
with `core::hint::spin_loop()`.  No stack unwinding — the stack may be corrupt.

### CI and scripts ✅

| Script / Workflow | Purpose |
|---|---|
| `scripts/run_qemu.sh` | Launch QEMU; `--gdb` adds `-s -S`, `--debug` skips `--release` |
| `scripts/size_report.sh` | Assert `.text+.rodata ≤ 512 KB`, `.data+.bss ≤ 256 KB` |
| `scripts/gdb_attach.sh` | Connect `riscv32-unknown-elf-gdb` to QEMU GDB stub on `:1234` |
| `scripts/boot_test.sh` | Boot QEMU, assert `"Ferret booting..."` in output |
| `.github/workflows/build.yml` | `cargo build --release` + boot test + size report |
| `.github/workflows/clippy.yml` | `cargo clippy -- -D warnings` (zero-warning policy) |

---

## Sprint 1 — Kernel Core

### TrapFrame (`kernel/src/context/mod.rs`) — Issue #12 ✅

In-memory snapshot of the full RISC-V machine state at trap entry.

| Field | Offset | Description |
|---|---|---|
| `regs: [usize; 32]` | 0x00–0x7C | x0–x31; x0 held as 0 for index uniformity |
| `mepc: usize` | 0x80 | Resume address for `mret` |
| `mstatus: usize` | 0x84 | Full snapshot including `MPP` (privilege-level return) |
| `mcause: usize` | 0x88 | Bit 31 = interrupt; bits 30:0 = cause code |
| `mtval: usize` | 0x8C | Faulting address for address exceptions; 0 otherwise |

Total: 36 × 4 = 144 bytes.  `#[repr(C)]` is mandatory — the assembly stub
accesses fields by these hardcoded byte offsets.  All 32 GPRs are saved
unconditionally so the interrupted context is always resume-transparent.
`mstatus` is saved in full to carry `MPP` for future user-mode returns
(Sprint 3 stretch).

### Trap entry stub (`kernel/src/context/trap.rs`) — Issue #13 ✅

`__trap_entry` is a `global_asm!` block placed in the `.trap` section.
`mtvec` is set to its address at boot in direct mode (bits[1:0] = 0).

Save sequence: `addi sp, sp, -144` → `sw x0–x31` at offsets 0–124 → CSR
reads of `mepc`/`mstatus`/`mcause`/`mtval` via t0 (already saved) into
offsets 128–140 → `mv a0, sp; call trap_handler`.

Restore sequence: reload `mepc`/`mstatus` via t0 → reload x31…x1 →
reload sp from offset 8 last (sp is used to address the frame until this
point) → `mret`.

`mcause` and `mtval` are not written back — they are read-only from software.

### CLINT driver (`kernel/src/clint.rs`) — Issue #14 ✅

MMIO base: `0x0200_0000`.  `mtime` at base+`0xBFF8`, `mtimecmp` at base+`0x4000`.

| Function | Description |
|---|---|
| `get_mtime() -> u64` | Two-read carry-safe read; retries if hi word changed across lo read |
| `set_mtimecmp(u64)` | Writes `u32::MAX` to hi first, then lo, then final hi — avoids spurious interrupt |
| `schedule_tick(u64)` | `set_mtimecmp(get_mtime() + cycles)` |
| `timer_isr()` | Increments `TICK_COUNT`, prints `[TICK N]`, calls `schedule_tick` |
| `ticks() -> u32` | Returns `TICK_COUNT` via `Ordering::Relaxed` |

`TICK_CYCLES = 10_000` = 1 ms at the QEMU virt 10 MHz timebase.

### Machine-mode interrupt enable (`kernel/src/main.rs`) — Issue #15 ✅

Boot sequence additions (in order):

1. `la t0, __trap_entry; csrw mtvec, t0` — direct-mode trap vector.
2. `clint::schedule_tick(TICK_CYCLES)` — arm first tick before enabling
   interrupts to prevent an immediate spurious MTIP.
3. `li t0, 0x80; csrs mie, t0` — set MTIE (mie[7]).
4. `li t0, 0x8; csrs mstatus, t0` — set global MIE (mstatus[3]).

`csrsi` is not used for MTIE because its immediate is 5 bits (0–31) and
`0x80 = 128` exceeds that; `csrs` with a register-held mask is used instead.

### Context switch (`kernel/src/context/switch.rs`) — Issue #16 ✅

`context_switch(old: *mut TrapFrame, new: *const TrapFrame)` saves the
callee-saved register set (ra, sp, s0–s11) and `mepc`/`mstatus` into `*old`,
then loads the same fields from `*new`.  Caller-saved registers are not
touched — the Rust compiler has spilled them at the call site.

`mstatus` is saved and restored in full to preserve the `MPP` field for
future privilege-level transitions (Sprint 3 stretch).

### Demo (`kernel/src/main.rs`) — Issue #17 ✅

A busy loop increments a `u32` counter and prints `"counter: N"` every
100 000 iterations.  The timer ISR fires at ~1 ms and prints `"[TICK N]"`.

Expected interleave (QEMU output):
```
[TICK 1]
counter: 100000
[TICK 2]
[TICK 3]
counter: 200000
```

Interleaved output confirms: `__trap_entry` saves state without corrupting
the counter, UART from the ISR does not corrupt task registers, and `mret`
resumes the loop at the correct instruction.
