# FerretOS

[![Build & Boot](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/build.yml/badge.svg)](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/build.yml)
[![Unit Tests](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/test.yml/badge.svg)](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/test.yml)
[![Clippy](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/clippy.yml/badge.svg)](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/clippy.yml)
[![Kani](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/kani.yml/badge.svg)](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/kani.yml)
[![Lean](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/lean.yml/badge.svg)](https://github.com/Bilal-Waraich/FerretOS/actions/workflows/lean.yml)

A `no_std` Rust microkernel for RISC-V systems with ≤ 256 KB RAM and no MMU. Tasks run in user mode, isolated by RISC-V PMP, and talk over capability-checked channels. The scheduler makes priority inversion over shared peripherals impossible by construction: priorities are fixed at build time from a contention graph, not adjusted at run time.

---

## Demo: a sensor-fusion pipeline

<img src="docs/figures/pipeline.svg" alt="Pipeline: sensor_reader (priority 3) sends on channel 0 to filter (priority 2), which sends on channel 1 to uart_logger (priority 1)" width="900"/>

Three user-mode tasks, each confined by PMP to its own 4 KB region, pass readings through kernel-owned channels. The logger is deliberately slow: 3 ms per line, plus a 25 ms flush every fifth line, longer than the sensor's 10 ms period. The sensor has the highest priority, so it preempts the logger in the middle of a flush and stays on time.

<img src="docs/figures/pipeline_timeline.svg" alt="Timeline from a QEMU run: the sensor runs every 10 ticks, including at ticks 50, 60, 100 and 110 while the logger is in the middle of a 25-tick flush" width="900"/>

The timeline above is drawn from a real QEMU run by [`scripts/plot_timeline.py`](scripts/plot_timeline.py). Serial output from the same run:

```
Capability check passed.
Channel check passed.
Task registry:
  task 0  base_pri=3  mip=3  eff_pri=3  excl_caps=0x00000002  req_caps=0x00000000  mem=[0x800b8000, 0x800b9000)
  task 1  base_pri=2  mip=2  eff_pri=2  excl_caps=0x00000000  req_caps=0x00000000  mem=[0x800b9000, 0x800ba000)
  task 2  base_pri=1  mip=1  eff_pri=1  excl_caps=0x00000001  req_caps=0x00000000  mem=[0x800ba000, 0x800bb000)
  task 3  base_pri=0  mip=0  eff_pri=0  excl_caps=0x00000000  req_caps=0x00000000  mem=[0x00000000, 0x00000000)
Interrupts enabled. Starting tasks.
[KERNEL] task 0 running in U-mode
[KERNEL] task 1 running in U-mode
[probe] log of kernel memory rejected (EFAULT)
[KERNEL] task 2 running in U-mode
[LOG] t=43 seq=4 release=40 raw=117 avg=75 latency=0 misses=0 dropped=0
[LOG] flush start=43 end=68
[LOG] t=71 seq=5 release=50 raw=166 avg=109 latency=0 misses=0 dropped=0
[LOG] t=74 seq=6 release=60 raw=199 avg=142 latency=0 misses=0 dropped=0
...
[LOG] summary: samples=100 max_latency=0 misses=0 dropped=0
```

Samples 5 and 6 were released at ticks 50 and 60, inside the flush, and still ran with latency 0. The logger prints them late, from the channel backlog, which is exactly what the channels are for. `task 3` is the kernel's idle task. The `[probe]` line shows the kernel refusing a task's request to print kernel memory.

The original three-task CA-PIP demo (L/M/H, where L inherits H's priority) is still available with `--features demo-hml`; see [Build and run](#build-and-run).

---

## Architecture

<img src="docs/figures/architecture.svg" alt="FerretOS architecture: build-time OML and CCG generation; user-mode tasks inside PMP regions; machine-mode kernel with trap path, syscalls, channels, CA-PIP scheduler and PMP; QEMU virt hardware" width="900"/>

**Capabilities.** Peripherals are zero-sized types (`ExclusiveCapability<UartCapability<0>>`), so ownership is checked by the compiler and costs nothing at run time. Each task's declaration also carries bitmasks of what it holds, needs, and which channel ends it owns. At boot the kernel halts with a diagnostic if two tasks claim the same exclusive peripheral or the same channel endpoint.

**CA-PIP scheduling.** At build time, `build.rs` builds a Capability Contention Graph from the OML declarations. An edge L → H means L holds a peripheral H needs. For each task it computes `MaxInheritedPriority`: the highest priority reachable in the graph. The kernel schedules on

```
effective_priority(T) = max(T.base_priority, MaxInheritedPriority(T))
```

which never changes at run time. A task holding a peripheral always runs at least as high as any task waiting for it, so a middle-priority task cannot get in between. The kernel binary contains no graph code; it copies a precomputed table at boot.

**Isolation.** Tasks run in U-mode. On every context switch the kernel reprograms PMP so the running task can read and execute flash and read and write its own 4 KB region, and nothing else: no kernel RAM, no other task, no MMIO. Traps land on a per-task kernel stack, and syscalls check every pointer against the caller's own memory before touching it.

**IPC.** Tasks share no memory, so channels live in the kernel: each is a ring of eight 16-byte messages. `send` blocks while the channel is full and `receive` while it is empty. When a send wakes a higher-priority receiver, the receiver runs immediately rather than at the next timer tick.

Design rationale and proofs: [FERRET.md](FERRET.md). Roadmap: [FerretEvolution.md](FerretEvolution.md). Implementation notes and measurements: [docs/internals.md](docs/internals.md).

---

## Status

| Phase | Sprint | Theme | Status |
|---|---|---|---|
| I | 0–5 | Boot, interrupts, memory regions, capabilities, CA-PIP, OML | ✅ v0.1.0 |
| I | 6 | Polish and demo | 🟡 H/M/L demo recording (#56) open |
| II | 7 | Graph computed at build time, out of the kernel binary | ✅ |
| II | 8 | U-mode tasks, syscall validation, per-task PMP, PMP fault test | ✅ |
| II | 9 | Capability-checked blocking channels | ✅ |
| II | 10 | Sensor-fusion pipeline ✅ · README ✅ · threat model (#80), recording (#92) | 🟡 |
| II | 11 | Lean: no-inversion theorem ✅ · scheduler step model (#90), SRP corollary (#81) | 🟡 |
| III | 12 | RV32IMC portability, ESP32-C3 in emulation | 🔲 |
| III | 13 | Execution-time tracing and deadline checks in CI | 🔲 |

Phase III is emulation-only: there is no physical board in this project, and nothing is claimed as validated on silicon.

---

## Build and run

### Prerequisites

- **Rust**: `rustup` installs the pinned nightly automatically from `rust-toolchain.toml`.
- **QEMU with RISC-V**:
  ```bash
  brew install qemu                      # macOS
  sudo apt-get install qemu-system-riscv # Debian/Ubuntu (qemu-system-misc on older releases)
  ```

```bash
git clone --recurse-submodules https://github.com/Bilal-Waraich/FerretOS
cd FerretOS
```

### Run

```bash
cargo build --release
./scripts/run_qemu.sh                      # sensor pipeline; Ctrl-A X to exit

cargo build --release --features demo-hml
./scripts/run_qemu.sh                      # original L/M/H CA-PIP demo

cargo build --release --features pmp
./scripts/run_qemu.sh                      # pipeline with per-task PMP isolation
```

Without `pmp`, tasks still run in U-mode but PMP grants them the whole address space. Enable `pmp` to get isolation; CI runs both builds.

### Test

```bash
./scripts/host_test.sh                     # unit tests on the host (default + pmp)

cargo build --release
bash tests/integration/boot_test.sh        # boot + pipeline timing checks

cargo build --release --features demo-hml
bash tests/integration/boot_test.sh hml    # L/M/H priority-inheritance ordering

bash tests/integration/pmp_fault_test.sh   # rogue store traps with PMP, lands without
bash scripts/switch_latency.sh             # context-switch cost, default vs pmp
./scripts/size_report.sh                   # flash/RAM budgets

cargo clippy -- -D warnings
(cd kernel && cargo kani --default-unwind 17 -Z unstable-options --ignore-global-asm)
(cd lean && lake exe cache get && lake build)
```

To regenerate the timeline figure: `cargo build --release && python3 scripts/plot_timeline.py`. To debug with GDB, run `./scripts/run_qemu.sh --gdb` in one terminal and `./scripts/gdb_attach.sh` in another.

To change the task set, edit `tasks/*.oml`. Building with the OML transpiler present (`./scripts/build_oml.sh`) regenerates `kernel/src/generated/` and the priority tables. The generated files are committed, so the kernel builds without OML.

---

## How it is checked

| Check | What it establishes | Where |
|---|---|---|
| Unit tests | Scheduler queue, build-time graph generator (cross-checked against the in-kernel algorithm), PMP encoding, syscall validation, channel block/wake | `scripts/host_test.sh` |
| Boot test | Tasks run in U-mode; the pipeline has 0 misses and 0 drops with latency ≤ 1 tick, including samples released inside logger flushes | `tests/integration/boot_test.sh` |
| H/M/L ordering | No M or H run starts inside L's critical section | `boot_test.sh hml` |
| PMP fault test | A U-mode store to kernel memory raises a store access fault with PMP and succeeds without it | `tests/integration/pmp_fault_test.sh` |
| Kani | Graph construction is in bounds; conflict detectors always catch a double claim; the priority queue keeps heap order; channel rings never exceed capacity and are FIFO with no loss or duplication | `kernel/src/kani_proofs.rs` |
| Lean 4 | If a holder blocks a waiter in the contention graph, the holder's inherited priority is at least the waiter's | `lean/FerretOS/` |

All of these run in CI on every push and pull request. CI also fails if the committed generated files differ from what OML produces.

### Measurements (QEMU, `-icount`)

| | Default | `pmp` |
|---|---|---|
| Context switch, average | 587 instructions | 631 instructions |
| Context switch, worst case | 817 instructions | 861 instructions |
| Flash / RAM used | 18 KB / 32 KB | — |

The budget is 1,000 cycles per switch. QEMU is not cycle-accurate, so switch cost is counted in retired instructions, which roughly track cycles on a simple in-order core. It is a proxy, not a cycle measurement.

---

## Limitations

- **Emulation only.** Everything runs on QEMU `virt`; timing figures are instruction counts, not cycles.
- **Channel waits do not raise priorities.** Only peripheral contention feeds the CCG. A high-priority task blocked on a full channel waits for its lower-priority consumer, and a middle-priority task can delay that consumer. The pipeline avoids this by sending downhill with `try_send`. See [docs/internals.md](docs/internals.md#blocking-channels--issues-76-77-).
- **Static system.** No dynamic tasks, no capability delegation or revocation. The firmware image is trusted as a whole.
- **The Lean proof covers the model, not the Rust code.** The Rust implementation is cross-checked against the model by unit tests and Kani, not proved equal to it.
- **PMP is opt-in** (`--features pmp`). The default build runs tasks in U-mode but grants them all memory.

---

## Design decisions

**No heap.** Every kernel structure is statically sized by `MAX_TASKS`, `MAX_PERIPHERALS` and `MAX_CHANNELS` in `config.rs`. Nothing can run out of memory at run time, and execution time stays bounded, which keeps worst-case analysis tractable.

**Priorities at build time.** Every input to the contention graph is known at compile time, so `build.rs` computes the priorities on the host, and its generator is unit-tested like kernel code. Moving it out cost no ROM, because LTO had already removed the unused boot-time code, but it removes any chance of the graph code running on the target.

**Kernel stacks, not task stacks, for traps.** M-mode stores bypass PMP. Saving a trap frame at a task-chosen stack pointer would let a task direct kernel writes anywhere, so each task gets a small kernel stack instead.

**Copying channels.** With per-task PMP there is no memory two tasks can both touch, so channels copy messages through the kernel. Messages are small (16 bytes) and the copy is bounded.

**`required_cap_mask` vs `exclusive_cap_mask`.** "Holds" and "needs" are separate masks. Graph edges come from `L.exclusive & H.required`, so the double-claim detector and the contention graph never conflict.

---

## Ecosystem position

| | **FerretOS** | RTIC | Hubris | Tock |
|---|---|---|---|---|
| Model | Microkernel | Concurrency framework | Microkernel | Kernel + processes |
| Isolation | Rust types + RISC-V PMP | Rust lifetimes | ARM MPU | ARM MPU / RISC-V PMP |
| Scheduling | CA-PIP (build-time contention graph) | SRP (compile-time ceilings) | Fixed priority, synchronous IPC | Round-robin / priority |
| Priority inversion | Prevented by construction (peripherals) | Prevented by construction | Avoided by IPC design | — |
| Task configuration | OML DSL | Rust proc-macros | Static TOML | Dynamic loading |
| Verification | Kani + Lean model | — | — | — |

RTIC is the closest relative: both are static and both prevent inversion by construction. FerretOS adds a microkernel boundary, with user-mode tasks, PMP isolation, syscalls and channels, around the same idea. For SRP's resource ceilings and how they relate to CA-PIP, see [FERRET.md](FERRET.md#ca-pip-and-srp-mathematical-relationship).

---

## Repository layout

```
FerretOS/
├── kernel/
│   ├── build.rs              — OML regeneration, build-time CCG + MIP tables
│   ├── build/ccg_gen.rs      — graph generator (shared with host unit tests)
│   ├── memory.x              — flash, kernel RAM, task RAM (TASKRAM) regions
│   └── src/
│       ├── lib.rs            — the `ferret` kernel library (host-testable)
│       ├── main.rs, boot.rs  — RISC-V entry point and boot sequence
│       ├── demo/             — pipeline.rs (default), hml.rs (--features demo-hml)
│       ├── context/          — trap frame, trap entry/exit, per-task kernel stacks
│       ├── scheduler/        — CA-PIP dispatch, ready queue, block/wake
│       ├── syscall.rs, sys.rs — kernel and task sides of the syscall ABI
│       ├── ipc/              — channel ring buffer, kernel channel table
│       ├── memory/           — regions, stacks + canaries, task registry, PMP
│       ├── capability/       — ZST capabilities, boot-time conflict detectors
│       ├── generated/        — committed OML output and MIP tables
│       └── kani_proofs.rs    — Kani harnesses
├── tasks/                    — task.oml (schema), pipeline_tasks.oml, demo_tasks.oml
├── lean/                     — Lean 4 model and proofs
├── oml/                      — OML transpiler (submodule)
├── tests/integration/        — boot_test.sh, pmp_fault_test.sh
├── scripts/                  — run_qemu, host_test, size_report, switch_latency, plot_timeline, build_oml
├── docs/                     — internals, algorithm, OML schema, figures
├── FERRET.md                 — Phase I design and proofs
└── FerretEvolution.md        — Phase II/III roadmap
```

---

## Constants (QEMU `virt`)

| | |
|---|---|
| Flash | `0x8000_0000`, 512 KB |
| Kernel RAM | `0x8008_0000`, 224 KB |
| Task RAM | `0x800B_8000`, 32 KB (4 KB per task) |
| UART | `0x1000_0000` |
| CLINT | `0x0200_0000` (`mtimecmp` `0x0200_4000`) |
| Timer tick | 1 ms |
| Time quantum | 5 ticks |
| `MAX_TASKS` / `MAX_CHANNELS` | 16 / 8 |
