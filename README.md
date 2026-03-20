# FerretOS

![Build](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/build.yml/badge.svg)
![Clippy](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/clippy.yml/badge.svg)

> A memory-safe, capability-aware microkernel for constrained RISC-V hardware. **Work in progress — Sprint 0 of 6.**

---

## What is this?

FerretOS is a `no_std` Rust microkernel targeting RISC-V systems with ≤256 KB RAM and no MMU. Memory safety is enforced by the type system. Priority inversion is structurally impossible via a proactive capability-aware scheduler (CA-PIP). Task declarations are written in OML and transpiled to Rust at build time.

See [FERRET.md](FERRET.md) for the full design document.

---

## Status

| Sprint | Goal | Status |
|--------|------|--------|
| 0 | Toolchain + QEMU boot | ✅ done |
| 1 | Interrupts + context switch | 🔲 not started |
| 2 | Memory safety layer | 🔲 not started |
| 3 | Capability system | 🔲 not started |
| 4 | CA-PIP scheduler | 🔲 not started |
| 5 | OML integration | 🔲 not started |
| 6 | Polish + demo | 🔲 not started |

---

## Target Hardware

| Property | Value |
|----------|-------|
| ISA | RISC-V RV32IMAC |
| RAM | ≤ 256 KB |
| Flash | ≤ 512 KB |
| MMU | None |
| Emulator | QEMU `virt` (riscv32) |
| Rust target | `riscv32imac-unknown-none-elf` |

---

## Prerequisites

- Rust nightly (pinned via `rust-toolchain.toml` — `rustup` handles this automatically)
- QEMU with RISC-V support: `brew install qemu` / `apt-get install qemu-system-misc`

---

## Build

```bash
cargo build --release
```

## Run

```bash
./scripts/run_qemu.sh
# Press Ctrl-A X to exit QEMU
```

Expected output:
```
====================================
  Ferret booting...
====================================
FerretOS v0.1.0 — Sprint 0 (bare-metal bring-up)
```

## Debug (GDB)

```bash
./scripts/run_qemu.sh --gdb
# In another terminal:
./scripts/gdb_attach.sh
```

---

## Architecture

> Diagram — Sprint 6

---

## Contributing

See the collaboration guide in [FERRET.md](FERRET.md).
