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

---

## Phase II

### Runtime CCG removed from the kernel binary — Issue #84 ✅

Since #42 the boot path only copies `MAX_INHERITED_PRIORITIES` (computed by
`kernel/build/ccg_gen.rs`) into the task registry. The in-kernel reference
implementation (`scheduler/ccg.rs`, `scheduler/mip.rs`) is now compiled only
under `cfg(test)` and `cfg(kani)`, where it cross-checks the build-time
generator and backs the Kani harnesses.

**ROM delta: 0 bytes.** Measured with `scripts/size_report.sh` on the release
build before and after the change:

| Section | Before | After |
|---|---|---|
| `.text` | 9434 | 9434 |
| `.rodata` | 1920 | 1920 |

The release profile already uses fat LTO, which discarded the never-called
graph code; gating the modules makes its absence a property of the source
rather than of the optimiser. `size_report.sh` enforces it: the build fails if
`llvm-nm` finds `CapabilityContentionGraph`, `compute_mip_values` or
`compute_and_store_mip` symbols in the kernel ELF. (The check matches symbol
names, so it would miss code that was fully inlined into a caller; with the
modules compiled out of non-test builds that cannot happen.)

`size_report.sh` also now counts `.data` toward the flash budget — its initial
values are stored in flash and copied to RAM at reset — and looks for
`llvm-size`/`llvm-nm` in the Rust sysroot.

### Preemptive multitasking and the trap path — Issues #103, #85 ✅

Every trap enters `__trap_entry`, which swaps `sp` with `mscratch` to land on
the running task's **kernel stack** (one 1 KB stack per registry slot, in
kernel `.bss`), saves a 144-byte `TrapFrame` at its top, and calls
`trap_handler(frame) -> frame`. The handler returns the frame to resume; the
exit path loads `sp` from it, sets `mscratch = frame + 144` (that task's
kernel stack top) and executes `mret`. Returning another task's saved frame
*is* the context switch.

The frame is kept off the task's own stack because tasks run in U-mode and
M-mode stores bypass PMP: a frame written at a task-chosen `sp` could
overwrite arbitrary kernel memory.

OML tasks start with `mstatus.MPP = U`; the kernel's idle task runs in M-mode
because `wfi` is illegal in U-mode on QEMU. A lowest-priority PMP entry
(entry 15, NAPOT over the whole address space, RWX) lets U-mode execute at
all; per-task entries take precedence where they match.

### Syscall ABI — Issues #103, #86 ✅

Tasks call the kernel with `ecall` (`mcause` 8 from U-mode, 11 from M-mode).
Wrappers live in `kernel/src/sys.rs`; the dispatcher in `kernel/src/syscall.rs`.

| Register | Role |
|---|---|
| `a7` | syscall number |
| `a0`, `a1` | arguments |
| `a0` | return value, or an error code |

| # | Name | Arguments | Returns | Notes |
|---|---|---|---|---|
| 0 | `yield` | — | 0 | Requeues the caller behind equal-priority peers. |
| 1 | `sleep` | `a0` = ticks (min 1) | 0 | Caller is `Blocked` until the tick arrives. |
| 2 | `log` | `a0` = address, `a1` = length | bytes written | Requires the console capability (`shared_cap_mask` bit 31). Length capped at 256. |
| 3 | `now` | — | ticks since boot | 1 tick = 1 ms. |
| 4 | `send` | `a0` = channel, `a1` = address, `a2` = length (≤ 16), `a3` = flags | 0 | Requires the sender endpoint (`sends_mask`). Blocks while full unless `a3 = FLAG_NONBLOCK`. |
| 5 | `receive` | `a0` = channel, `a1` = buffer, `a2` = buffer size, `a3` = flags | message length | Requires the receiver endpoint (`receives_mask`). Blocks while empty unless `FLAG_NONBLOCK`. Copy truncated to the buffer. |

| Error | Value | Meaning |
|---|---|---|
| `ENOSYS` | `usize::MAX` | Unknown syscall number. |
| `EPERM` | `usize::MAX - 1` | Caller lacks the required capability. |
| `EFAULT` | `usize::MAX - 2` | Buffer not accessible by the caller. |
| `EAGAIN` | `usize::MAX - 3` | Would block (non-blocking variant). |
| `EINVAL` | `usize::MAX - 4` | Argument out of range (no such channel, message > 16 bytes). |

**Validation.** The kernel never dereferences a task pointer until the whole
buffer `[addr, addr + len)` (overflow-checked) lies inside memory the caller
may read itself: its own stack, or the read-only flash image `[_stext,
_sidata)` (code and string constants). The task's declared `MemoryRegion` is
not yet accepted, because demo regions are not backed by dedicated linker
sections and can overlap kernel data (addressed with per-task PMP, #87). A
`log` of a kernel address returns `EFAULT`; the boot test checks this.

### Per-task PMP isolation — Issues #87, #89 ✅

RAM is split in `kernel/memory.x`: kernel RAM (224 KB at `0x8008_0000`) and
**TASKRAM** (32 KB at `0x800B_8000`). Each task's stack is placed in a
`.task_ram.<slot>` section; the sections are 4 KB aligned and laid out in slot
order, so task `n`'s 4 KB region is `0x800B_8000 + n·0x1000`. The OML region
declarations match this, and boot halts with `MEMORY LAYOUT ERROR` if a task's
stack is not inside its declared region. Moving the stacks out of `.data` cut
the flash image from 25 KB to 14 KB.

With `--features pmp`:

| Entry | Covers | U-mode |
|---|---|---|
| 0 | flash, NAPOT 512 KB | R + X |
| 1–2 | running task's region (NAPOT if power-of-two and aligned, else TOR) | R + W |

Entries 1–2 are rewritten in `mark_running` on every dispatch. Kernel RAM,
MMIO and other tasks' regions match no entry. `tests/integration/pmp_fault_test.sh`
shows a U-mode store to a kernel variable trapping (`store access fault`,
`mtval` = target address) with PMP, and succeeding without it.

### Context-switch latency — Issues #41, #88 ✅

Latency is measured in **retired instructions** (`minstret`), from trap
handler entry through the new task's PMP setup, plus the 84 instructions of
the assembly stub (45 entry, 39 exit) that the handler cannot observe. `mtime`
is unsuitable: QEMU advances it with host wall-clock time.

`scripts/switch_latency.sh` (QEMU `-icount shift=0`, 300 timer-driven switches
per build; identical across runs):

| Build | min | avg | max |
|---|---|---|---|
| default | 524 | 587 | 817 |
| `pmp` | 568 | 631 | 861 |

PMP costs **+44 instructions** per switch on average (two `pmpaddr` writes
and three `pmpcfg0` read-modify-writes). Both builds stay within the
1000-instruction budget. **Caveat:** QEMU is not cycle-accurate; on a simple
in-order RV32 core one instruction is roughly one cycle, so these are a proxy
for the Sprint 4 cycle budget, not a cycle measurement.

### Blocking channels — Issues #76, #77 ✅

Tasks in U-mode share no memory, so channels live in the kernel
(`ipc/kernel.rs`): `MAX_CHANNELS` (8) slots, each a `Channel<Message, 8>` ring
buffer of messages up to 16 bytes. `send` copies a message out of the
caller's memory, `receive` copies one into the caller's buffer, after the
same buffer validation as `log` (`receive` additionally rejects flash: the
buffer must be writable by the task).

**Blocking is restartable.** When a channel is full (send) or empty
(receive), the kernel rewinds `mepc` to the `ecall`, marks the task
`Blocked(Send(ch))` / `Blocked(Receive(ch))`, and dispatches the next task.
The counterpart operation wakes the (single, by SPSC) waiting task; if it now
outranks the caller, the kernel switches to it immediately rather than at the
next tick. On resume the task re-executes the `ecall`, which succeeds.

**Limitation — channel waits are not in the CCG.** Following the design in
`FerretEvolution.md`, only capability contention creates CCG edges. A task
blocked on a channel waits for its peer, but neither inherits the other's
priority. If a high-priority task blocks on a full channel whose consumer has
lower priority, a task of intermediate priority can delay the consumer and
therefore the high-priority task: priority inversion through IPC is possible.
Designs avoid it by flowing data from higher to lower priority with no task
between the stages (as the sensor pipeline does), by sizing channels so the
producer never blocks, or by using `try_send`. Modelling channel waits as CCG
edges (sender → receiver for empty, receiver → sender for full) would close
the gap at the cost of raising priorities along every bidirectional channel.
