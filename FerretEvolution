# Evolution of Ferret OS

> From software-guaranteed prototype to formally verified, hardware-enforced physical deployment.

---

## Table of Contents

1. [Phase II Thesis](#phase-ii-thesis)
2. [Motivation](#motivation)
3. [Technical Architecture Additions](#technical-architecture-additions)
   - [Zero-Cost Scheduling: Compile-Time CCG](#zero-cost-scheduling-compile-time-ccg)
   - [Defense in Depth: RISC-V PMP Hardware Isolation](#defense-in-depth-risc-v-pmp-hardware-isolation)
   - [IPC Primitives: Capability-Gated Message Passing](#ipc-primitives-capability-gated-message-passing)
   - [Compelling Demo: Sensor Fusion Pipeline](#compelling-demo-sensor-fusion-pipeline)
   - [Threat Model and Security Boundary Document](#threat-model-and-security-boundary-document)
   - [Algorithmic Proofs: Formal Verification via Lean 4](#algorithmic-proofs-formal-verification-via-lean-4)
4. [Development Plan (Phase II)](#development-plan-phase-ii)
   - [Sprint 7 — Zero-Cost Graph (Weeks 15–16)](#sprint-7--zero-cost-graph-weeks-1516)
   - [Sprint 8 — Hardware Defense (Weeks 17–18)](#sprint-8--hardware-defense-weeks-1718)
   - [Sprint 9 — IPC Primitives (Weeks 19–20)](#sprint-9--ipc-primitives-weeks-1920)
   - [Sprint 10 — Sensor Fusion Demo (Weeks 21–22)](#sprint-10--sensor-fusion-demo-weeks-2122)
   - [Sprint 11 — Algorithmic Proofs (Weeks 23–24)](#sprint-11--algorithmic-proofs-weeks-2324)
5. [Phase II Issues Breakdown](#phase-ii-issues-breakdown)
6. [Phase III: Physical Deployment and Execution Proofs](#phase-iii-physical-deployment-and-execution-proofs)
   - [Phase III Thesis](#phase-iii-thesis)
   - [Sprint 12 — Silicon Reality (Weeks 25–26)](#sprint-12--silicon-reality-weeks-2526)
   - [Sprint 13 — Mathematical Certainty: Automated WCET Analysis (Weeks 27–28)](#sprint-13--mathematical-certainty-automated-wcet-analysis-weeks-2728)
   - [Phase III Issues Breakdown](#phase-iii-issues-breakdown)

---

## Phase II Thesis

The baseline Ferret OS implementation successfully proves that a capability-aware microkernel can structurally eliminate priority inversion without relying on an MMU or dynamic memory. However, the baseline retains boot-time computational overhead, tasks cannot communicate with one another, and the system's security claims remain informal assertions rather than auditable arguments.

**Phase II** transitions Ferret OS from an isolated proof-of-concept into a coherent, communicating, formally argued system. It eliminates boot-time latency by shifting graph mathematics entirely to the Rust build system, enforces compiler guarantees with physical hardware traps (RISC-V PMP), gives tasks a capability-gated channel for coordination, anchors the system's security claims in a precise written threat model, and introduces interactive theorem proving (Lean 4) to mathematically guarantee the scheduler's structural integrity.

---

## Motivation

### The Problem with Boot-Time Overhead

In Phase I, the Capability Contention Graph (CCG) is built dynamically in RAM when the system boots. While acceptable for a prototype, this wastes ROM space (storing graph traversal logic) and delays system startup. In critical embedded systems, boot time must be instantaneous. Because every variable in Ferret OS is known at compile time via OML, performing this math at boot is an architectural redundancy.

### The Limits of Software Safety

Rust's type system is a flawless static analysis tool, but it cannot prevent physical runtime anomalies: a cosmic ray flipping a bit in RAM, a misconfigured DMA controller overwriting memory, or an errant physical peripheral. To achieve defense-in-depth, Ferret's software guarantees must be backed by a hardware failsafe.

### Tasks That Cannot Communicate

Phase I tasks are islands. They run, they hold capabilities, they are scheduled — but they cannot pass data to one another. A real microkernel's value is in composing isolated tasks into a cooperative system. Without IPC, Ferret cannot demonstrate the most fundamental use case of a microkernel: running a pipeline of tasks where each stage reads from the previous one. Phase II closes this gap with a minimal, capability-gated message-passing primitive, and then demonstrates it in a concrete sensor fusion scenario.

### Informal Security Claims

Ferret's Phase I documentation makes strong security assertions — priority inversion is structurally impossible, task memory is isolated, capabilities cannot alias — but these claims have no corresponding document that specifies what the system protects, against what threat actors, under what assumptions, and where it explicitly does not provide guarantees. Without a threat model, reviewers (and master's programme admissions committees) cannot distinguish rigorous claims from wishful thinking. Phase II adds this document.

### The Algorithmic Guarantee

Simply observing that a task "doesn't seem to cause priority inversion" is unacceptable for hard real-time, safety-critical systems. We must shift from empirical observation to mathematical proof, formally demonstrating that the CA-PIP algorithm prevents priority inversion under all possible execution interleavings, not just the ones tested.

---

## Technical Architecture Additions

### Zero-Cost Scheduling: Compile-Time CCG

Phase II removes the CCG logic from the kernel binary entirely. The burden of graph construction and `MaxInheritedPriority` calculation is shifted to the host machine via the Rust build system.

1. **The Build Pipeline:** The `build.rs` script, which transpiles OML files, is expanded. After generating the Rust `TaskDescriptor` types, it constructs the directed CCG internally during `cargo build`.
2. **Const Generation:** The build script calculates the `MaxInheritedPriority` for each task and emits a static Rust file containing a `const` array of these values.
3. **Runtime Result:** The kernel boots, maps the generated `const` array into ROM, and the scheduler reads effective priorities in O(1) time with absolute zero setup latency. The kernel no longer contains graph traversal code.

**Correctness note:** Shifting CCG construction to `build.rs` makes the build script a critical correctness artifact. A bug here produces silently wrong `MaxInheritedPriority` values with no runtime error. The SRP cross-validation test from Sprint 4 (issue #36b) becomes *mandatory* in this phase: generated values must be verified against hand-computed SRP ceilings on the same task set before the runtime graph traversal logic is removed from the kernel.

### Defense in Depth: RISC-V PMP Hardware Isolation

RISC-V Physical Memory Protection (PMP) provides hardware-level access control to physical memory. Ferret maps its compile-time `MemoryRegion` ZST abstractions directly to the CPU's PMP registers (`pmpcfg` and `pmpaddr`).

- **Privilege Separation:** The kernel executes in RISC-V Machine mode (`M-mode`), granting it full access to all memory and CSRs. Tasks are downgraded to execute in User mode (`U-mode`).
- **Hardware Traps:** Before context switching to a task, the scheduler configures the PMP registers to exclusively grant Read/Write/Execute access to that specific task's memory bounds.
- **Performance consideration:** PMP register writes are not free. Writing up to 16 CSRs on every context switch on a 10MHz RV32 core will consume cycles. Sprint 8 must measure context switch latency with PMP enabled and verify the total remains within the ≤1000 cycle budget established in Sprint 4. The number of PMP regions per task may need to be bounded to meet this constraint.
- **The Result:** If a hardware glitch or a rogue pointer attempts an out-of-bounds access, the CPU physically blocks the transaction, throwing an `Instruction/Data Access Fault` and returning control cleanly to the microkernel.

```
Rust type system (compile-time)         ← Primary safety layer
         +
RISC-V PMP registers (runtime)          ← Hardware enforcement layer
         =
Defense-in-depth isolation
```

### IPC Primitives: Capability-Gated Message Passing

Phase I tasks are isolated — they hold capabilities and run on schedule but cannot exchange data. Phase II introduces a minimal inter-task communication mechanism consistent with Ferret's capability model.

**Design:** A `Channel<T, CAP>` is a statically allocated bounded queue (fixed capacity `CAP`, element type `T`) with typed sender and receiver capability tokens:

```rust
// Zero-sized capability tokens — no runtime overhead
pub struct SenderCapability<const ID: usize>;
pub struct ReceiverCapability<const ID: usize>;

// Statically allocated channel — no heap
pub struct Channel<T, const CAP: usize> {
    buffer: [MaybeUninit<T>; CAP],
    head: AtomicUsize,
    tail: AtomicUsize,
}
```

A task that holds `SenderCapability<0>` may write to `Channel<SensorReading, 8>` with ID 0. A task holding `ReceiverCapability<0>` may read from it. No other task can touch the channel at the type level. The capability conflict detector already checks exclusive capabilities at boot; channel capabilities integrate into this existing check.

**OML schema extension:** Channel declarations are added to `.oml` task definitions alongside peripheral and memory declarations:

```
Task sensor_reader {
    ...
    sends: Channel(0, SensorReading)
}

Task filter {
    ...
    receives: Channel(0, SensorReading)
    sends: Channel(1, FilteredReading)
}
```

**Blocking semantics:** Send blocks if the channel is full; receive blocks if empty. The scheduler transitions the calling task to `Blocked` and records the channel ID in its descriptor. When the channel state changes (a consumer reads, freeing space, or a producer writes, making data available), the scheduler transitions the relevant waiting task back to `Ready`. This integrates directly with the existing `TaskState` enum.

### Compelling Demo: Sensor Fusion Pipeline

Phase I's 3-task UART demo proves the scheduler works in isolation. Phase II replaces it with a pipeline demo that makes the microkernel *useful* rather than merely correct. The sensor fusion pipeline is a natural fit: it exercises the scheduler, the capability system, and the new IPC primitives simultaneously, and it maps to a real embedded use case.

**Pipeline architecture:**

```
[Task: sensor_reader]  →  Channel<RawReading, 8>  →  [Task: filter]
                                                           │
                                              Channel<FilteredReading, 4>
                                                           │
                                                   [Task: uart_logger]
```

- **`sensor_reader`** (priority 3, highest): Simulates a hardware sensor by producing `RawReading` values on a 10ms deadline. Holds `SenderCapability<0>` and `UartCapability<SENSOR>`.
- **`filter`** (priority 2, medium): Reads raw values, applies a simple moving average, emits `FilteredReading`. Holds `ReceiverCapability<0>` and `SenderCapability<1>`.
- **`uart_logger`** (priority 1, lowest): Reads filtered values and writes formatted output to UART. Holds `ReceiverCapability<1>` and `UartCapability<LOG>`.

The demo UART output will show timestamps alongside each logged value. Because `uart_logger` holds `UartCapability<LOG>` exclusively, CA-PIP ensures it cannot hold up `sensor_reader` — the scheduler's preemption decisions derived from the CCG make starvation of the high-priority sensor task structurally impossible. The recorded demo will highlight this explicitly, showing that even when `uart_logger` is mid-write, `sensor_reader` preempts cleanly and resumes on schedule.

### Threat Model and Security Boundary Document

Phase II adds `docs/threat_model.md` — a short, precise document establishing what Ferret protects, against what adversaries, and where its guarantees explicitly end. This is not marketing copy; it is an engineering artifact that makes Ferret's security claims auditable.

**Structure:**

1. **Assets:** What Ferret protects — task memory regions, exclusive peripheral access, scheduling fairness.
2. **Threat actors:** The assumed adversary model — a buggy task (not a malicious one), hardware faults, DMA misconfiguration. Ferret explicitly does *not* claim to protect against a malicious task with the ability to exploit hardware vulnerabilities, or against attacks requiring physical access to the board.
3. **Guarantees provided:**
   - Memory isolation: a task cannot read or write another task's `MemoryRegion` — enforced at compile time by the type system, and at runtime by PMP (with `feature="pmp"`).
   - Capability exclusivity: no two tasks can hold the same `ExclusiveCapability` — enforced at boot time by the capability allocator.
   - Priority inversion freedom: a low-priority task cannot starve a high-priority task — enforced proactively by CA-PIP using precomputed `MaxInheritedPriority`.
4. **Guarantees not provided:**
   - No protection against cosmic-ray bit-flips or hardware faults outside PMP reach.
   - No dynamic capability delegation or revocation — capabilities are lexical and compile-time only.
   - No support for dynamic task loading — the threat model assumes a trusted, monolithic firmware image.
5. **Trusted computing base:** The kernel core (`kernel/src/`), the OML transpiler (as a build-time dependency), and the Rust compiler. Anything outside this boundary is not trusted.

### Algorithmic Proofs: Formal Verification via Lean 4

While Rust guarantees memory safety, it cannot prove algorithmic correctness. Phase II introduces formal verification of the CA-PIP scheduling algorithm using the **Lean 4** interactive theorem prover. Rather than verifying the Rust implementation line-by-line, this effort formally verifies the *algorithmic specification* of Ferret's scheduler.

**The Verification Pipeline:**

1. **State Modeling:** The system's state is modeled in Lean 4 as a pure functional data structure (Task Sets, Capability Graph, Ready/Blocked Queues).
2. **Transition Semantics:** Timer ticks and resource requests are modeled as state transition functions (`Step(State_n) → State_n+1`).
3. **The Core Theorem:** Lean 4 is used to prove that the CA-PIP step function can never transition into an inverted state.

The primary invariant proved in Lean 4 guarantees that the effective priority of the executing task is always greater than or equal to the base priority of any ready task. Because effective priority is statically derived from the compile-time CCG, this proof guarantees the precomputed graph completely bounds all possible runtime contention.

The Lean 4 proof files live in `proofs/` at the repo root and are cross-referenced from `docs/ccg_algorithm.md`, which translates the formal proof calculus into readable mathematical notation. This connects directly to the existing `Lean4-FP` portfolio work, forming a coherent narrative across the GitHub profile: the Lean4 repo demonstrates foundational proof engineering; the Ferret proofs demonstrate it applied to a real systems problem.

---

## Development Plan (Phase II)

### Sprint 7 — Zero-Cost Graph (Weeks 15–16)

**Goal:** Strip CCG computation from the runtime kernel. Boot time drops to near-zero; ROM footprint shrinks.

**Deliverables:**
- Expand `build.rs` to parse capability mappings and construct the CCG at compile time.
- Implement BFS/DFS algorithm inside the build script to compute `MaxInheritedPriority`.
- Emit `src/generated/priorities.rs` containing a `const` array of precomputed values.
- Cross-validate all generated `MaxInheritedPriority` values against hand-computed SRP ceilings before stripping runtime logic.
- Strip all graph/node structures from the `ferret` kernel crate.
- Measure and log ROM size reduction before and after in `size_report.sh` output.

**Technical notes:**
- The build script is now a correctness artifact, not just a code generator. Add unit tests inside `build.rs` using Rust's `#[test]` in a `tests` module within the script — these run on the host and verify CCG construction on synthetic task sets with known correct answers.
- `println!("cargo:rerun-if-changed=tasks/")` already in place from Sprint 5; extend to also rerun if `build.rs` itself changes.

---

### Sprint 8 — Hardware Defense (Weeks 17–18)

**Goal:** Implement physical isolation via RISC-V PMP. Context switch latency remains within budget with PMP enabled.

**Deliverables:**
- Configure RISC-V `mstatus` register to support `M-mode` to `U-mode` privilege transitions.
- Write a PMP driver to map `MemoryRegion<START, END>` structs to NAPOT/TOR PMP configurations.
- Update the context switcher (`switch.S`) to update PMP registers upon task swap.
- Measure context switch latency with PMP enabled; confirm total remains ≤1000 cycles on RV32 at 10MHz.
- Integration test: a user task intentionally attempts an out-of-bounds memory access; verify the kernel catches the trap cleanly and logs a diagnostic over UART.
- Gate the entire PMP layer behind `feature = "pmp"` so the QEMU baseline remains unaffected.

**Technical notes:**
- PMP register writes (`pmpcfg`, `pmpaddr`) are CSR writes — use `csrw` / `csrrs` inline assembly. Budget approximately 2 CSR writes per PMP entry, per context switch.
- NAPOT encoding requires region sizes to be powers of two and naturally aligned. If a task's `MemoryRegion` is not NAPOT-compatible, fall back to TOR (Top Of Range) mode using two consecutive PMP entries.
- The kernel's own memory (`.text`, `.rodata`, stack, interrupt vectors) must be configured as M-mode only before the first task is run.

---

### Sprint 9 — IPC Primitives (Weeks 19–20)

**Goal:** Tasks can communicate. A producer task and a consumer task exchange data through a capability-gated channel. The scheduler correctly transitions tasks between Ready and Blocked on channel state changes.

**Deliverables:**
- `Channel<T, CAP>` type: statically allocated, fixed-capacity, lock-free bounded queue.
- `SenderCapability<ID>` and `ReceiverCapability<ID>` ZST tokens.
- Boot-time channel capability conflict check integrated into the existing capability allocator.
- OML schema extended: `sends` and `receives` fields in task declarations.
- Scheduler integration: `send()` blocks if full; `receive()` blocks if empty; wakes waiting tasks on state change.
- Unit tests: channel send/receive under capacity, blocking behaviour, wake-on-write.

**Technical notes:**
- `AtomicUsize` for head/tail indices — requires the `A` extension in RV32IMAC. Confirm QEMU virt supports this; it does for the standard `virt` machine.
- The channel buffer is `[MaybeUninit<T>; CAP]` — avoid constructing `T` for empty slots. Use `ptr::write` / `ptr::read` to move values in and out.
- `MAX_CHANNELS` is a const in `config.rs` alongside `MAX_TASKS`. Channel descriptors join the static registry at link time.

---

### Sprint 10 — Sensor Fusion Demo (Weeks 21–22)

**Goal:** The complete sensor fusion pipeline runs in QEMU. The demo is recorded and attached to the repository. The threat model document is complete.

**Deliverables:**
- All three pipeline tasks (`sensor_reader`, `filter`, `uart_logger`) implemented and running.
- UART output shows timestamped filtered readings with visible evidence of CA-PIP preemption behaviour (sensor_reader consistently meets its 10ms deadline even under logger load).
- Recorded QEMU demo (`asciinema` or screen capture) committed to `docs/demo/`.
- `docs/threat_model.md` complete: assets, threat actors, guarantees provided, guarantees not provided, TCB definition.
- README updated to feature the sensor fusion pipeline as the primary demo narrative, replacing the abstract 3-task scenario from Phase I.

**Technical notes:**
- The demo output should be human-readable: timestamps in milliseconds derived from CLINT `mtime`, task name prefix on each UART line, deadline miss counter (should remain zero throughout the recording).
- The threat model is a living document — flag it explicitly as Phase II scope in a header comment so Phase III additions (PMP hardening, physical silicon) have a natural extension point.

---

### Sprint 11 — Algorithmic Proofs (Weeks 23–24)

**Goal:** Mathematically prove the CA-PIP algorithm prevents priority inversion. Proof files are committed, CI type-checks them, and a human-readable translation is in the docs.

**Deliverables:**
- `proofs/Model.lean`: Definitions of Tasks, Capabilities, and the CCG using Lean 4 inductive types.
- `proofs/Scheduler.lean`: Pure functional implementation of the CA-PIP state transition logic.
- `proofs/Theorems.lean`: The formal proof that priority inversion is an unreachable state under CA-PIP.
- GitHub Actions workflow: `lean_check.yml` — runs `lake build` on the `proofs/` directory, fails if proofs do not type-check.
- `docs/ccg_algorithm.md` extended: a section translating the Lean 4 proof calculus into readable mathematical notation, cross-referenced from the proof files.

**Technical notes:**
- Use Lean 4 with Mathlib for standard library support (finite sets, graph reachability lemmas).
- The core invariant in `Theorems.lean`: `∀ s : SystemState, ∀ t : Task, t ∈ s.ready → effectivePriority(s.running) ≥ basePriority(t)`.
- Do not attempt to verify the Rust implementation directly — verify the algorithmic model. The connection between the model and the implementation is argued informally in `docs/ccg_algorithm.md`.

---

## Phase II Issues Breakdown

| Title | Label | Sprint |
|---|---|---|
| Implement compile-time CCG algorithm in `build.rs` | `build`, `scheduler` | Sprint 7 |
| Add host-side unit tests to `build.rs` for CCG correctness | `test`, `build` | Sprint 7 |
| Generate `priorities.rs` const array | `build`, `oml` | Sprint 7 |
| Cross-validate generated MIP values against SRP ceilings | `test`, `scheduler` | Sprint 7 |
| Strip runtime CCG logic from kernel | `kernel`, `cleanup` | Sprint 7 |
| Implement RISC-V `M-mode` to `U-mode` transition in `switch.S` | `kernel`, `security` | Sprint 8 |
| Map `MemoryRegion` ZSTs to PMP NAPOT/TOR registers | `memory`, `security` | Sprint 8 |
| Measure context switch latency with PMP enabled | `scheduler`, `perf` | Sprint 8 |
| Write PMP violation trap integration test | `test`, `security` | Sprint 8 |
| Define `Channel<T, CAP>` with static allocation | `ipc`, `kernel` | Sprint 9 |
| Define `SenderCapability<ID>` and `ReceiverCapability<ID>` ZSTs | `ipc`, `capability` | Sprint 9 |
| Integrate channel capabilities into boot-time conflict detector | `ipc`, `capability` | Sprint 9 |
| Extend OML schema with `sends` and `receives` fields | `ipc`, `oml` | Sprint 9 |
| Implement scheduler blocking/waking on channel send/receive | `ipc`, `scheduler` | Sprint 9 |
| Write IPC unit tests (capacity, blocking, wake) | `test`, `ipc` | Sprint 9 |
| Implement `sensor_reader` task (simulated sensor, 10ms deadline) | `demo` | Sprint 10 |
| Implement `filter` task (moving average over channel) | `demo` | Sprint 10 |
| Implement `uart_logger` task (formatted UART output from channel) | `demo` | Sprint 10 |
| Record and commit QEMU sensor fusion demo | `docs`, `demo` | Sprint 10 |
| Write `docs/threat_model.md` | `docs`, `security` | Sprint 10 |
| Update README to feature sensor fusion pipeline as primary demo | `docs` | Sprint 10 |
| Write `proofs/Model.lean`: Tasks, CCG, system state | `verification`, `math` | Sprint 11 |
| Write `proofs/Scheduler.lean`: CA-PIP transition function | `verification`, `math` | Sprint 11 |
| Write `proofs/Theorems.lean`: priority inversion unreachability proof | `verification`, `math` | Sprint 11 |
| Add `lean_check.yml` GitHub Actions workflow | `ci`, `verification` | Sprint 11 |
| Extend `docs/ccg_algorithm.md` with proof translation | `docs`, `math` | Sprint 11 |

---

## Phase III: Physical Deployment and Execution Proofs

### Phase III Thesis

Phase II delivers a formally argued, communicating, hardware-isolated Ferret OS — but still running in QEMU emulation. Phase III addresses the two remaining gaps between a research prototype and a genuinely deployable system: running on physical silicon, and providing mathematical proof of timing correctness. These are deliberately deferred to Phase III because they carry substantial engineering risk that would destabilise a Phase II timeline if included prematurely.

---

### Sprint 12 — Silicon Reality (Weeks 25–26)

**Goal:** Ferret OS boots on a physical ESP32-C3 development board. The sensor fusion pipeline demo from Phase II runs on real hardware with physical UART output.

**ISA compatibility note:** The ESP32-C3 implements RV32IMC — it lacks the Atomic (`A`) extension present in Ferret's QEMU target (`riscv32imac`). This is a meaningful architectural difference. Every use of `AtomicUsize` in the IPC layer (channel head/tail indices) must be audited and replaced with critical-section-based equivalents using the `critical-section` crate. This audit should be completed before any physical board work begins.

**Deliverables:**
- ISA audit: identify all uses of atomic operations in the kernel and IPC layer; document replacement strategy.
- Custom linker script `esp32c3.ld` mapping to the ESP32-C3's physical SRAM and Flash layout.
- Physical clock tree initialisation: PLL configuration, watchdog disable, system clock setup via register writes.
- Hardware-specific UART driver for the ESP32-C3 peripheral memory map.
- `probe-rs` integration for flashing `ferret.elf` over JTAG/USB.
- Sensor fusion pipeline demo running on physical hardware with LED indicators for task preemption events.

**Technical notes:**
- ESP32-C3 SRAM: 400KB total (split between instruction and data); Flash: 4MB. Ferret's 256KB RAM constraint is met comfortably, but the memory map layout is non-standard — linker script work is non-trivial.
- The ESP-IDF ROM bootloader expects a specific partition table in Flash. The simplest approach is to produce a raw binary with no partition table and flash it directly to address `0x0` using `probe-rs`, bypassing the ESP-IDF boot sequence entirely.
- Physical LED demo: assign one GPIO per task; a GPIO pulse on each context switch provides a logic-analyser-visible record of scheduler behaviour. This is more compelling on video than raw UART numbers.

---

### Sprint 13 — Mathematical Certainty: Automated WCET Analysis (Weeks 27–28)

**Goal:** Each task's worst-case execution time is computed statically and compared against its OML-declared deadline. The CI pipeline fails if any task violates its deadline.

**Tooling note:** OTAWA's RISC-V backend is academic-quality and can be difficult to configure reliably against a `no_std` binary with custom linker sections. Sprint 13 should begin with a lighter-weight approach — manual cycle-counting via CLINT timestamps logged to UART — to establish empirical baselines and validate that declared deadlines are realistic before investing in static analysis tooling. OTAWA (or a comparable tool such as `platin` for LLVM-based RISC-V analysis) is introduced only once the empirical groundwork is in place. The CI-fails-on-WCET-violation story is compelling, but only if the tooling produces reliable results.

**Deliverables:**
- CLINT-based cycle counter instrumentation: each task logs entry/exit timestamps to a static circular buffer; a diagnostic task drains the buffer to UART after each scheduling epoch.
- Empirical WCET baselines established for all three pipeline tasks across 1000 scheduling cycles in QEMU and on physical hardware.
- OTAWA (or `platin`) containerised in a Docker image and integrated into CI as an optional `wcet_check.yml` workflow.
- Analysis script: extracts deadline values from `.oml` files and compares against static analysis output; fails the build on violations.
- `docs/wcet_analysis.md`: methodology, tool configuration, known limitations, and comparison of empirical vs. static bounds.

**Technical notes:**
- Static WCET analysis requires loop bound annotations on any loop whose iteration count is not statically obvious. Ferret's `no_std` design — no dynamic allocation, bounded queues, fixed task set — makes most loop bounds trivially static. Document the annotations required.
- The gap between empirical worst case and static WCET bound is expected to be significant (static tools are conservative). Document this gap explicitly; it is not a failure of the analysis but a feature of its soundness guarantee.
- If OTAWA does not produce reliable results for the target binary format, `cargo-call-stack` provides a lighter-weight alternative that proves the absence of dynamic dispatch and unbounded recursion — a weaker but still meaningful property.

---

### Phase III Issues Breakdown

| Title | Label | Sprint |
|---|---|---|
| Audit all atomic operations for RV32IMC compatibility | `kernel`, `hardware` | Sprint 12 |
| Replace atomics with critical-section equivalents where required | `kernel`, `hardware` | Sprint 12 |
| Write ESP32-C3 custom linker script | `hardware` | Sprint 12 |
| Implement ESP32-C3 physical clock and watchdog initialisation | `hardware` | Sprint 12 |
| Rewrite UART driver for ESP32-C3 memory map | `hardware` | Sprint 12 |
| Integrate `probe-rs` for JTAG/USB flashing | `tooling`, `hardware` | Sprint 12 |
| Demonstrate sensor fusion demo on physical hardware with LEDs | `demo`, `hardware` | Sprint 12 |
| Instrument tasks with CLINT cycle counters | `perf`, `verification` | Sprint 13 |
| Establish empirical WCET baselines in QEMU and on hardware | `perf`, `verification` | Sprint 13 |
| Containerise OTAWA (or `platin`) for RISC-V static analysis | `ci`, `verification` | Sprint 13 |
| Integrate WCET static analysis into `wcet_check.yml` CI workflow | `ci`, `verification` | Sprint 13 |
| Write analysis script: OML deadlines vs. static WCET output | `ci`, `oml` | Sprint 13 |
| Write `docs/wcet_analysis.md` | `docs`, `verification` | Sprint 13 |
