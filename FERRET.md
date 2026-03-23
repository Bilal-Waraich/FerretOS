# Ferret OS

> A memory-safe, capability-aware microkernel for constrained hardware.

---

## Table of Contents

1. [Thesis](#thesis)
2. [Motivation](#motivation)
3. [What Ferret Is Not](#what-ferret-is-not)
4. [Core Design Philosophy](#core-design-philosophy)
5. [Known Design Tradeoffs](#known-design-tradeoffs)
6. [Technical Architecture](#technical-architecture)
   - [Target Platform](#target-platform)
   - [Kernel Architecture](#kernel-architecture)
   - [Memory Model](#memory-model)
   - [Capability System](#capability-system)
   - [Defense in Depth: RISC-V PMP](#defense-in-depth-risc-v-pmp)
   - [Scheduler: Capability-Aware Priority Inheritance](#scheduler-capability-aware-priority-inheritance)
   - [CA-PIP and SRP: Mathematical Relationship](#ca-pip-and-srp-mathematical-relationship)
   - [OML Integration](#oml-integration)
7. [Ecosystem Position](#ecosystem-position)
8. [Repository Structure](#repository-structure)
9. [Development Plan](#development-plan)
   - [Sprint 0 — Foundations](#sprint-0--foundations-weeks-12)
   - [Sprint 1 — Kernel Core](#sprint-1--kernel-core-weeks-34)
   - [Sprint 2 — Memory Safety Layer](#sprint-2--memory-safety-layer-weeks-56)
   - [Sprint 3 — Capability System](#sprint-3--capability-system-weeks-78)
   - [Sprint 4 — Scheduler](#sprint-4--scheduler-weeks-910)
   - [Sprint 5 — OML Integration](#sprint-5--oml-integration-weeks-1112)
   - [Sprint 6 — Polish & Demo](#sprint-6--polish--demo-weeks-1314)
10. [GitHub Issues Breakdown](#github-issues-breakdown)
11. [Collaboration Guide](#collaboration-guide)
12. [Glossary](#glossary)

---

## Thesis

Most embedded RTOSes are written in C and rely on the programmer to avoid memory errors. Most memory-safe kernels are too large for constrained hardware. **Ferret** is a microkernel written in Rust targeting RISC-V systems with under 256KB RAM, where memory safety is guaranteed by the type system and priority inversion is impossible by construction. Tasks declare their resource requirements in OML; the scheduler uses those declarations to make provably safe preemption decisions.

---

## Motivation

### The Problem with Existing Embedded OSes

The embedded RTOS landscape in 2025 is dominated by C: FreeRTOS, Zephyr, RIOT, ChibiOS. These are mature, battle-tested systems. They are also fundamentally unsafe. Memory errors — buffer overflows, use-after-free, dangling pointers — are the leading cause of embedded system failures and security vulnerabilities. The programmer is the last line of defence, and the programmer is fallible.

The standard response to this in the broader systems community has been Rust. Rust's ownership and borrowing model eliminates entire classes of memory errors at compile time, with no garbage collector and no runtime overhead. Projects like Redox OS have proven that a full OS in safe Rust is viable. The problem is that Redox, and similar efforts, target general-purpose hardware. They assume megabytes of RAM, a full MMU, and a rich peripheral ecosystem.

Constrained hardware — microcontrollers in the Cortex-M and RISC-V families, systems with 64KB to 256KB of RAM, no MMU, no virtual memory — has been largely left out of the memory-safety conversation. The assumption is that "you can't afford Rust on a microcontroller." Ferret challenges that assumption directly.

### The Problem with Priority Inversion

Beyond memory safety, there is a second endemic problem in embedded scheduling: **priority inversion**. This occurs when a high-priority task is blocked waiting for a resource held by a low-priority task, while a medium-priority task preempts the low-priority task and runs freely. The result is that the highest-priority task — the one that most urgently needs CPU time — is the one that waits longest. This is not a theoretical concern. The Mars Pathfinder mission in 1997 experienced repeated system resets caused by a classic priority inversion in its VxWorks scheduler: a low-priority meteorological task held a lock on an information bus, was preempted by a medium-priority communications task, and a high-priority bus management task was then blocked indefinitely. The resulting systemic delay caused the watchdog timer to expire, triggering catastrophic system resets that nearly doomed the mission.

Most RTOSes address this with **Priority Inheritance Protocol (PIP)**: when a low-priority task holds a resource a high-priority task is waiting for, the low-priority task temporarily inherits the higher priority. This works, but it is implemented reactively — the scheduler discovers the inversion after it has already begun. PIP has further structural weaknesses: it does not prevent deadlocks when lock dependencies are circular, and it is susceptible to **chain blocking**, where a task can be blocked multiple times by different lower-priority tasks holding different resources.

The **Priority Ceiling Protocol (PCP)** and its variant **Immediate Ceiling Priority Protocol (ICPP)** address some of these flaws by associating a static ceiling priority with each resource — defined as the highest priority of any task that may use it — and bumping a task to that ceiling immediately upon resource acquisition. This prevents deadlocks and bounds blocking to a single critical section, but still requires dynamic priority manipulation at runtime, consuming CPU cycles and complicating Worst-Case Execution Time (WCET) analysis.

Ferret's scheduler is **proactive**. Because every task must declare its resource requirements at registration time — using OML — the scheduler has a complete static picture of potential contention before any task runs. It can compute the worst-case priority inheritance graph at boot time and use that graph to make preemption decisions that are safe by construction, not by recovery.

### Why OML

OML (Object Markup Language) is a language-neutral DSL for describing data structures and classes. It transpiles to multiple target languages. In the context of Ferret, OML serves as the task declaration interface: a human-readable, language-agnostic format in which developers describe what a task needs — memory regions, peripherals, timing constraints, priority — and OML transpiles those declarations into the Rust types the kernel consumes.

This has two concrete benefits. First, it decouples task authorship from kernel internals: a developer writing a task does not need to know the internal representation of a `TaskDescriptor` in the kernel; they write OML and the build system handles the rest. Second, it makes the kernel configuration portable by construction: the same OML task definitions could in principle transpile to C for a different target, or to a formal specification for verification.

OML is not the reason Ferret exists. But it is a natural and honest fit for the problem Ferret is solving, and it gives OML a motivated, real-world application domain.

### Why This Matters Beyond the Project

The combination of properties Ferret targets — memory safety, minimal footprint, static capability analysis, language-agnostic configuration — is directly relevant to the direction embedded systems and IoT are moving. As constrained devices become more networked and more security-critical, the cost of memory unsafety increases. Ferret is a proof-of-concept that safe, aware scheduling is achievable on hardware that the safety community has largely ignored.

---

## What Ferret Is Not

It is important to be precise about scope.

- **Ferret is not a general-purpose OS.** It does not aim to run a shell, a filesystem, or user applications in the traditional sense. It is a microkernel: the minimum substrate for safe, scheduled execution of tasks on constrained hardware.
- **Ferret is not a replacement for FreeRTOS or Zephyr.** It is a research-quality proof of concept with a specific thesis. Production deployment is not the goal.
- **Ferret is not a Rust tutorial.** The codebase is expected to use advanced Rust features — `no_std`, `unsafe` blocks where hardware interaction genuinely requires them (clearly documented and audited), custom allocators, const generics for compile-time task configuration — and is not designed to be pedagogically gentle.
- **Ferret does not require an MMU.** This is an explicit design constraint. Ferret targets systems without virtual memory hardware. Memory protection is enforced by Rust's type system and by the capability system, not by hardware page tables.

---

## Core Design Philosophy

**1. Safety by construction, not by convention.**
Every invariant that can be enforced by the Rust type system must be. `unsafe` blocks are permitted only at hardware boundaries — register reads/writes, interrupt handlers — and every such block is individually documented with a safety argument.

**2. Static over dynamic.**
Ferret favours decisions made at compile time or boot time over decisions made at runtime. The number of tasks, their priorities, and their resource declarations are all known before the first task runs. The scheduler operates on a precomputed contention graph.

**3. Minimal and auditable.**
The kernel core should be small enough that a careful reader can audit its entire memory safety argument in an afternoon. Complexity belongs in userspace (or in OML tooling), not in the kernel.

**4. Explicit over implicit.**
If a task needs a memory region, it must declare it. If a task holds a peripheral, the type system must reflect that. Nothing is implicitly shared. Everything is explicit in the task descriptor.

---

## Known Design Tradeoffs

Ferret is a research-quality project with a deliberate thesis. The following are known tradeoffs that represent conscious decisions, not oversights. They are documented here so that contributors understand the boundaries of the design space.

### Software Isolation vs. Hardware Enforcement

Ferret relies on Rust's ownership model to enforce task isolation — no MMU, no MPU in the baseline design. This is correct and intentional for the 256KB RAM target where hardware isolation adds complexity and platform coupling. However, it is a real limitation: the Rust borrow checker is a static analysis tool. It cannot protect against runtime hardware failures, cosmic ray bit-flips, or DMA peripherals that write to unauthorized memory. Systems like **Hubris** (Oxide Computer) and **Tock OS** pair software safety with ARM Cortex-M MPU enforcement for true defense-in-depth. On RISC-V, the equivalent is **Physical Memory Protection (PMP)** — see the [Defense in Depth section](#defense-in-depth-risc-v-pmp) for a detailed discussion of how Ferret can optionally integrate PMP as a hardening layer.

The tradeoff is explicit: baseline Ferret trusts its codebase entirely. This is correct for a monolithic, audited firmware image on a constrained target. It would be insufficient for systems that load unverified third-party code at runtime — which is outside Ferret's stated scope.

### Boot-Time vs. Compile-Time CCG

The Capability Contention Graph is currently built at boot time. Every variable required to compute it — task priorities, capability declarations — is known at compile time and is immutable. A future optimisation is to shift CCG construction entirely into the Cargo build system, emitting flat static arrays of precomputed `MaxInheritedPriority` values directly into the binary. This would eliminate boot latency, reduce ROM footprint, and strip the graph traversal logic from the kernel entirely — converging on the approach used by the **RTIC framework's Stack Resource Policy** implementation.

This is noted as a **Sprint 4 stretch goal** and a natural future contribution. The baseline implementation builds the CCG at boot deliberately, because doing so makes the algorithm visible and auditable at runtime — which has pedagogical and debugging value in this phase of the project.

### OML as External DSL

OML is a language-neutral transpiler and not a native Rust abstraction. This introduces toolchain friction: developers must install the OML binary, context-switch between OML and Rust syntax, and maintain the transpiler as a build dependency. The Rust ecosystem's native alternative would be procedural macros (`proc_macro`), as demonstrated by RTIC's `#[rtic::app]` macro. Ferret uses OML because it is a collaborator-built tool with a real application domain, and one goal of this project is to give OML a motivated, systems-level use case. This is a deliberate choice, not a default. Contributors who prefer a pure-Rust configuration path are encouraged to open a discussion — a TOML-based fallback configuration is a valid future extension.

---

## Technical Architecture

### Target Platform

| Property | Value |
|---|---|
| ISA | RISC-V (RV32IMAC) |
| RAM budget | ≤ 256KB |
| ROM/Flash budget | ≤ 512KB |
| MMU | None |
| MPU/PMP | RISC-V PMP (optional hardening layer — see below) |
| Primary emulation target | QEMU `virt` machine (RISC-V 32-bit) |
| Secondary physical target | ESP32-C3 or similar (stretch goal) |
| Rust target triple | `riscv32imac-unknown-none-elf` |
| Build system | Cargo workspace + OML build script |

QEMU is the primary development and demonstration target. The `virt` machine provides UART output, a timer (CLINT), and enough peripherals to demonstrate the scheduler. Physical hardware is a stretch goal for Sprint 6.

---

### Kernel Architecture

Ferret is a **microkernel**. The kernel itself provides exactly four services:

1. **Memory management** — static allocation of memory regions to tasks at boot time; no heap in the kernel core.
2. **Interrupt handling** — vectored interrupt dispatch, timer interrupt for preemption.
3. **Task lifecycle** — task registration, context switching, task states (Ready, Running, Blocked, Suspended).
4. **Capability-aware scheduler** — the novel component; described in detail below.

Everything else — drivers, communication primitives, higher-level abstractions — lives outside the kernel core, either as privileged tasks or as libraries.

```
┌─────────────────────────────────────────────────┐
│                   OML Toolchain                 │  ← Build-time
│         (task declarations → Rust types)        │
└───────────────────┬─────────────────────────────┘
                    │ generates
┌───────────────────▼─────────────────────────────┐
│              Task Registry (static)             │  ← Boot-time
│    [ TaskDescriptor × N, CapabilityGraph ]      │
└───────────────────┬─────────────────────────────┘
                    │ consumed by
┌───────────────────▼─────────────────────────────┐
│                 Ferret Kernel                   │  ← Runtime
│  ┌────────────┐ ┌──────────┐ ┌───────────────┐ │
│  │  Scheduler │ │  Memory  │ │   Interrupt   │ │
│  │  (CA-PIP)  │ │  Manager │ │   Controller  │ │
│  └────────────┘ └──────────┘ └───────────────┘ │
└─────────────────────────────────────────────────┘
```

---

### Memory Model

Ferret operates without virtual memory. Memory safety is achieved through two mechanisms:

**1. Rust ownership at the type level.**
Memory regions are represented as owned types. A task that owns a `MemoryRegion<0x2000, 0x2FFF>` holds exclusive access to that range. The type system prevents two tasks from being assigned overlapping regions at compile time — the region allocator produces distinct non-overlapping typed handles, and no two handles with overlapping ranges can coexist in the type system.

**2. Static allocation.**
There is no dynamic memory allocation in the kernel. All task stacks, task descriptors, and kernel data structures are allocated statically at link time. The linker script enforces that total static allocation fits within the RAM budget. This means: no allocator bugs, no heap fragmentation, no OOM conditions at runtime.

The stack for each task is a statically sized array, typed as `Stack<N>` where `N` is specified in the task's OML descriptor. The kernel's memory manager assigns these stacks at boot and records their boundaries for stack overflow detection via a guard region pattern.

---

### Capability System

A **capability** in Ferret is a typed token representing exclusive or shared access to a hardware resource. Capabilities come in two flavours:

- **Exclusive capabilities** — only one task may hold this token at a time. Examples: `UartCapability<UART0>`, `GpioCapability<PIN_13>`.
- **Shared capabilities** — multiple tasks may read but not mutate. Examples: `RomRegion<0x8000, 0x9FFF>`.

Capabilities are declared in OML and exist as zero-sized types (ZSTs) in Rust — they have no runtime representation and therefore no runtime overhead. Their presence in a task's type signature is purely a compile-time contract.

```
// OML declaration
Task sensor_reader {
    priority: 3
    stack_size: 2048
    memory: Region(0x20000000, 0x20000FFF)
    peripheral: UART0
    deadline: 50ms
}

// Generated Rust (by OML build script)
pub struct SensorReaderDescriptor {
    pub priority: Priority<3>,
    pub stack: Stack<2048>,
    pub memory: MemoryRegion<0x20000000, 0x20000FFF>,
    pub peripheral: UartCapability<UART0>,
    pub deadline: Deadline<50>,
}
```

At boot, the kernel's capability allocator verifies that no two tasks declare overlapping exclusive capabilities. This is a boot-time check, not a runtime check — if it fails, the kernel halts with a diagnostic before any task runs.

---

### Defense in Depth: RISC-V PMP

Ferret's baseline relies on Rust's type system for task isolation — no hardware enforcement. For production hardening or higher-assurance targets, Ferret supports an optional **RISC-V Physical Memory Protection (PMP)** layer that maps the statically defined `MemoryRegion` types to hardware PMP registers during boot.

RISC-V PMP allows the hardware to partition physical memory into up to 64 configurable regions, each with independent Read/Write/Execute access permissions tied to the CPU's current privilege mode (Machine vs. User). When enabled in Ferret, each task's statically declared memory region is mapped 1:1 to a PMP entry at boot. If an unsafe block, a misconfigured DMA controller, or a hardware fault causes an out-of-bounds memory access, the PMP unit immediately traps the violation — transforming a potentially silent memory corruption into a clean, handleable fault that the microkernel can log and recover from.

```
Rust type system (compile-time)         ← Primary safety layer
         +
RISC-V PMP registers (runtime)          ← Hardware enforcement layer
         =
Defense-in-depth isolation
```

This dual-layer approach mirrors the architecture of **Tock OS** (ARM Cortex-M MPU) and **Keystone TEE** (RISC-V PMP for trusted execution environments). It is the standard for high-assurance confidential computing on RISC-V.

**Implementation notes:**
- PMP configuration registers (`pmpcfg0`–`pmpcfg15`, `pmpaddr0`–`pmpaddr63`) are written during the boot sequence, after the static `TaskRegistry` is finalised and before the first task runs.
- Each `MemoryRegion<START, END>` maps to a NAPOT (Naturally Aligned Power-Of-Two) or TOR (Top Of Range) PMP entry.
- The kernel's own memory is marked as Machine-mode only, inaccessible from User-mode tasks.
- PMP integration is gated behind a Cargo feature flag (`feature = "pmp"`) so the baseline kernel remains minimal and QEMU-compatible without it.

**PMP is a Sprint 3 stretch goal** (see issue #28a). It is not required for the core thesis demonstration but is the correct path if Ferret is to be deployed on physical hardware with untrusted peripherals.

---

### Scheduler: Capability-Aware Priority Inheritance

The scheduler is the novel core of Ferret. It implements a variant of the **Priority Inheritance Protocol (PIP)** that is **proactive rather than reactive**.

#### Standard PIP (reactive)

In standard PIP, when task H (high priority) attempts to acquire a resource held by task L (low priority), L's priority is raised to H's priority for the duration. This resolves the inversion but only after it has begun — the scheduler must detect the inversion at runtime.

#### Ferret's CA-PIP (proactive)

Because all task capabilities are declared statically in OML, Ferret builds a **Capability Contention Graph (CCG)** at boot time. The CCG is a directed graph where:

- Nodes are tasks.
- An edge from L → H with label `C` means: task L holds capability C which task H also requires.

From the CCG, the scheduler precomputes for each task its **maximum inherited priority** — the highest priority of any task that could be blocked waiting for a resource it holds. This value is stored in the task descriptor and used directly by the scheduler's preemption decision without any graph traversal at runtime.

```
Boot time:
  1. Parse all TaskDescriptors (generated from OML)
  2. Build Capability Contention Graph
  3. For each task T, compute MaxInheritedPriority(T)
     = max { priority(H) | H requires a capability held by T }
  4. Store MaxInheritedPriority in TaskDescriptor

Runtime scheduling:
  - When task T is running and holds capability C:
    effective_priority(T) = max(T.base_priority, T.max_inherited_priority)
  - Preemption decisions use effective_priority, not base_priority
  - No graph traversal. O(1) priority lookup.
```

The result: **priority inversion cannot occur at runtime**, not because the scheduler detects and corrects it, but because the scheduler's preemption decisions are computed from a model that makes inversion structurally impossible.

#### Scheduling Algorithm

Ferret uses a **preemptive fixed-priority scheduler** with round-robin tie-breaking at equal priorities. The ready queue is a statically sized priority queue (binary heap over a fixed array — no dynamic allocation). The timer interrupt triggers the scheduler at a configurable tick rate (default: 1ms).

On each timer tick:
1. Check if the current task has exceeded its time quantum.
2. If yes, move it to the back of its priority level.
3. Compute effective priorities for all ready tasks using precomputed MaxInheritedPriority.
4. Select the highest effective-priority ready task.
5. If different from the current task, context switch.

Context switching saves and restores the full RISC-V register file (32 general-purpose registers + CSRs relevant to the task) to/from the task's statically allocated context frame.

---

### CA-PIP and SRP: Mathematical Relationship

CA-PIP does not exist in isolation. It shares deep mathematical equivalency with the **Stack Resource Policy (SRP)**, the scheduling protocol underlying the **RTIC framework** — the most widely used Rust RTOS framework in production embedded systems today. Understanding the relationship clarifies both the novelty and the positioning of Ferret's approach.

**SRP in brief:** SRP assigns a static *priority ceiling* `π(r)` to each resource `r`, defined as the maximum base priority of any task that accesses it. The system maintains a running *system ceiling* `Π` = max of all priority ceilings of currently held resources. A task may only preempt if its base priority is strictly greater than `Π`. This prevents priority inversion, eliminates deadlocks, and bounds blocking to a single critical section — all provable from the static resource declarations.

RTIC achieves this with zero runtime overhead by evaluating all `π(r)` values entirely at compile time via Rust procedural macros, mapping the system ceiling directly to ARM's `BASEPRI` hardware register. Critical sections enter and exit in two machine instructions.

**Where CA-PIP diverges:** Both CA-PIP and SRP are proactive, both require static resource declarations, and both achieve O(1) priority lookups at runtime. The key architectural difference is that CA-PIP operates at the level of the **task graph** rather than the **resource ceiling**. Where SRP asks "what is the ceiling of this resource?", CA-PIP asks "what is the maximum priority of any task that could be blocked by *this task's* holdings, across all possible execution interleavings?" This is a strictly stronger guarantee: CA-PIP's `MaxInheritedPriority` is the ceiling of the ceiling, computed over the full CCG topology rather than per-resource.

In practice, for a system with non-overlapping resource graphs (which is typical in constrained embedded targets), CA-PIP and SRP converge to the same effective priority values. The meaningful difference emerges in systems with multi-hop dependency chains, where CA-PIP's transitive closure over the CCG catches contention that per-resource ceilings miss.

```
SRP ceiling for resource R:
  π(R) = max { priority(T) | T accesses R }

CA-PIP MaxInheritedPriority for task T:
  MIP(T) = max { priority(H) | H is reachable from T in CCG }
         = max over all transitive contention chains rooted at T
```

**Implication for Ferret:** This relationship means that Ferret's CCG algorithm can be validated against known SRP results — if the CCG is correct, MIP values should be verifiable by cross-checking against hand-computed SRP ceilings on the same task set. This is a useful correctness test to include in the Sprint 4 test suite.

**Future direction — compile-time CCG:** Since every input to the CCG computation is statically known at compile time, a natural evolution is to shift CCG construction from boot time into a Cargo build script, emitting `MaxInheritedPriority` as a `const` array. This would converge Ferret's implementation with RTIC's approach: zero boot-time computation, zero ROM overhead for graph traversal logic, and provable correctness before the binary is ever flashed. This is tracked as a **Sprint 4 stretch goal** (issue #36a).

### Worst-Case Execution Time (WCET)

Because Ferret forbids dynamic allocation and dynamic task creation, every variable required for formal WCET analysis is available at compile time. This makes Ferret an unusually tractable target for static WCET tools compared to general-purpose RTOSes.

WCET analysis computes a provable upper bound on the execution time of each task for every possible input, guaranteeing that hard real-time deadlines are met before deployment rather than discovered in testing. The combination of:
- Static task set (known at compile time)
- Bounded stack sizes (`Stack<N>` with compile-time N)
- O(1) scheduler preemption decisions
- No heap allocation (no allocator overhead)

...means that Ferret's task execution model is directly amenable to interval analysis tools like **OTAWA** or **AbsInt aiT** for RISC-V. Integrating WCET analysis as a CI step — failing the build if a task's computed WCET exceeds its declared deadline — is a **Sprint 6 stretch goal** (issue #50a) and would be a meaningful research contribution in its own right.

---

### OML Integration

OML is integrated into the Ferret build pipeline as a **Cargo build script** (`build.rs`). The flow is:

```
OML files (*.oml)
      │
      ▼
  oml transpiler (Rust binary, part of OML workspace)
      │
      ▼
  Generated Rust source (task descriptors, capability types)
      │
      ▼
  Cargo compiles generated source into kernel binary
      │
      ▼
  ferret.elf → loaded into QEMU
```

OML files live in `tasks/` at the repo root. Each `.oml` file defines one or more tasks. The build script watches these files, re-runs the transpiler on change, and places generated Rust in `src/generated/`. The generated files are committed to the repo so that the kernel can be built without the OML toolchain installed (though OML is required to modify task definitions).

This integration keeps a clean separation: the OML toolchain is a build-time dependency, not a runtime one. The kernel binary has no knowledge of OML — it sees only the generated Rust types.

---

## Ecosystem Position

Ferret occupies a specific and intentional niche in the 2026 embedded systems landscape. The table below positions it against its closest architectural peers.

| Feature | **Ferret OS** | Hubris (Oxide) | Tock OS | RTIC | seL4 |
|---|---|---|---|---|---|
| Primary language | Rust | Rust | Rust | Rust | C (formally verified) |
| Architectural model | Microkernel | Microkernel | Kernel + user processes | Concurrency framework | Capability microkernel |
| Memory isolation | Software (Rust ZSTs + lifetimes) | Hardware (ARM MPU) | Hardware (ARM MPU) | Software (Rust lifetimes) | Hardware (MMU/MPU) |
| Hardware isolation (optional) | RISC-V PMP (stretch) | ARM MPU (required) | ARM MPU (required) | None | MMU (required) |
| Scheduling algorithm | Proactive (CA-PIP) | Synchronous / fixed | Preemptive | Proactive (SRP) | Preemptive / MCS |
| Priority inversion mitigation | Precomputed effective priority (CCG) | None natively | Yield / callbacks | Static priority ceilings | Runtime priority inheritance |
| Task / resource definition | Compile-time (OML transpiler) | Compile-time (static config) | Dynamic / sideloading | Compile-time (Rust macros) | Dynamic creation |
| Dynamic task loading | No | No | Yes | No | Yes |
| RAM target | ≤ 256KB | ~2000 LOC kernel | ~64KB | Zero-cost abstractions | Scalable / variable |
| Formally verified | No (stretch: Lean4 invariants) | No | No | No | Yes (C source) |

### Reading the Table

**Ferret vs. RTIC:** The closest technical sibling. Both are proactive, both use static resource declarations, and both target constrained hardware. RTIC is a concurrency framework built on bare-metal ISRs; Ferret provides a full microkernel abstraction with formal task lifecycle management and an explicit hardware abstraction layer. CA-PIP and SRP are mathematically related but architecturally distinct — see the [CA-PIP and SRP section](#ca-pip-and-srp-mathematical-relationship).

**Ferret vs. Hubris:** Both reject dynamic task creation and rely on a fully static, compile-time-verified system image. Hubris mandates ARM MPU isolation for every driver component, making it appropriate for production industrial firmware. Ferret targets hardware that may lack MPU capabilities entirely, relying on Rust's type system as the primary isolation mechanism with PMP as an optional layer.

**Ferret vs. Tock OS:** Tock supports dynamic application loading — firmware updates over the air without rebooting the kernel. Ferret explicitly rejects this model. Tock targets ~64KB systems with ARM MPU; Ferret targets up to 256KB on RISC-V without assuming hardware isolation. Different tradeoffs for different problem statements.

**Ferret vs. seL4:** seL4 is the gold standard for formally verified capability-based microkernels. Its capability model is dynamic — capabilities are physical objects in memory, evaluated at runtime, supporting delegation and revocation between processes. Ferret's ZST capabilities are lexical and compile-time only, which is faster and smaller but fundamentally incompatible with dynamic, multi-trust-domain applications. Ferret's niche is the trusted monolithic firmware image where seL4's runtime machinery would be wasted overhead.

---

## Repository Structure

```
ferret/
├── .github/
│   ├── workflows/
│   │   ├── build.yml          # Build + QEMU smoke test on every push
│   │   └── clippy.yml         # Rust linting
│   └── ISSUE_TEMPLATE/
│       ├── bug_report.md
│       └── feature.md
├── kernel/                    # Kernel crate (no_std)
│   ├── src/
│   │   ├── main.rs            # Entry point, boot sequence
│   │   ├── scheduler/
│   │   │   ├── mod.rs
│   │   │   ├── ccg.rs         # Capability Contention Graph
│   │   │   ├── pip.rs         # CA-PIP implementation
│   │   │   └── queue.rs       # Static priority queue
│   │   ├── memory/
│   │   │   ├── mod.rs
│   │   │   ├── region.rs      # MemoryRegion types
│   │   │   └── stack.rs       # Static stack allocation
│   │   ├── capability/
│   │   │   ├── mod.rs
│   │   │   ├── types.rs       # Capability ZSTs
│   │   │   └── allocator.rs   # Boot-time capability conflict check
│   │   ├── interrupt/
│   │   │   ├── mod.rs
│   │   │   ├── handler.rs     # Vectored interrupt dispatch
│   │   │   └── timer.rs       # CLINT timer interface
│   │   ├── context/
│   │   │   ├── mod.rs
│   │   │   └── switch.S       # RISC-V context switch (assembly)
│   │   └── generated/         # OML-generated task descriptors
│   │       └── .gitkeep
│   ├── build.rs               # OML build script
│   └── Cargo.toml
├── tasks/                     # OML task definitions
│   ├── sensor_reader.oml
│   └── logger.oml
├── oml/                       # OML submodule or workspace member
│   └── (OML transpiler codebase)
├── linker/
│   └── ferret.ld              # Linker script for RISC-V target
├── scripts/
│   ├── run_qemu.sh            # Launch QEMU with ferret.elf
│   ├── gdb_attach.sh          # GDB over QEMU for debugging
│   └── size_report.sh         # Report binary + RAM usage
├── docs/
│   ├── architecture.md        # This document
│   ├── ccg_algorithm.md       # Detailed CCG construction + proof sketch
│   ├── memory_model.md        # Memory safety argument
│   └── figures/               # Architecture diagrams
├── tests/
│   └── integration/           # QEMU-based integration tests
├── Cargo.toml                 # Workspace root
├── Cargo.lock
├── rust-toolchain.toml        # Pin nightly toolchain version
├── .gitignore
└── README.md
```

---

## Development Plan

### Sprint 0 — Foundations (Weeks 1–2)

**Goal:** Development environment is fully working. A minimal binary boots in QEMU and prints to UART. Nothing interesting yet — but the entire toolchain is proven.

**Deliverables:**
- QEMU boots `ferret.elf` and prints `"Ferret booting..."` over UART
- Rust target `riscv32imac-unknown-none-elf` building cleanly
- Linker script correctly places sections within RAM/ROM budget
- `run_qemu.sh` and `size_report.sh` working
- GitHub Actions CI running on every push (build + QEMU boot check)
- README skeleton committed

**Technical notes:**
- Use `riscv-rt` crate for startup code (reset handler, stack pointer initialisation) or write a minimal startup in assembly — document the choice
- UART output via memory-mapped I/O to QEMU's 16550 UART at `0x10000000`
- Panic handler: write panic message to UART, then `loop {}`
- `.cargo/config.toml` configures the target and runner (`qemu-system-riscv32`)

---

### Sprint 1 — Kernel Core (Weeks 3–4)

**Goal:** Interrupts work. The timer fires. Context exists as a concept.

**Deliverables:**
- Machine-mode interrupt handler registered and functional
- CLINT timer interrupt firing at configurable rate (default 1ms)
- Trap frame struct defined: captures all 32 RISC-V GPRs + relevant CSRs (`mepc`, `mstatus`, `mcause`)
- Single "task" (no scheduler yet) running in a loop, interrupted by timer, resuming correctly
- Assembly context save/restore routine (`switch.S`) written and tested

**Technical notes:**
- RISC-V machine-mode interrupts: `mtvec` set to trap handler address, `mstatus.MIE` enabled
- CLINT base address on QEMU `virt`: `0x2000000`; `mtimecmp` at `0x2004000`
- Trap handler must be `#[naked]` in Rust — standard function prologue would corrupt the register state it is trying to save
- Context frame is a fixed-size struct stored at the bottom of each task's static stack
- **Note on PMP:** If the PMP hardening layer (see [Defense in Depth](#defense-in-depth-risc-v-pmp)) is to be adopted, the trap handler infrastructure established here must support Machine-mode to User-mode privilege transitions. Retrofitting privilege separation into a completed context switcher is difficult — the foundation should be laid now even if PMP registers are not yet written. The `mstatus.MPP` field and `mret` instruction are the relevant primitives.

---

### Sprint 2 — Memory Safety Layer (Weeks 5–6)

**Goal:** Memory regions are typed. Two tasks have statically non-overlapping memory. The type system enforces it.

**Deliverables:**
- `MemoryRegion<START, END>` type with const generic bounds
- Compile-time check that two `MemoryRegion`s do not overlap (via const eval)
- `Stack<N>` type: statically allocated `[u8; N]` with alignment
- Static task registry: fixed-size array of `TaskDescriptor`s, populated at link time
- `size_report.sh` verifies total static allocation ≤ 256KB

**Technical notes:**
- Const generic overlap check: `const _: () = assert!(END_A < START_B || END_B < START_A)` — compile error if violated
- Stack alignment: RISC-V requires 16-byte stack alignment; `Stack<N>` must be `#[repr(align(16))]`
- TaskDescriptor at this stage: `{ priority: u8, stack: *mut Stack<N>, memory: MemoryRegion<S,E>, state: TaskState }`
- `TaskState`: `Ready | Running | Blocked | Suspended` — plain enum, no heap

---

### Sprint 3 — Capability System (Weeks 7–8)

**Goal:** Capabilities exist as types. No two tasks can hold the same exclusive capability. Boot-time conflict detection works.

**Deliverables:**
- `UartCapability<N>`, `GpioCapability<PIN>` ZST capability types defined
- `ExclusiveCapability<T>` and `SharedCapability<T>` wrappers
- Boot-time capability allocator: iterates all `TaskDescriptor`s, checks for exclusive conflicts, halts with diagnostic if found
- At least two tasks with non-conflicting capabilities running (no scheduler yet — round-robin manually triggered by timer)
- Unit tests for capability conflict detection logic
- **(Stretch — issue #28a)** Map `MemoryRegion` types to RISC-V PMP registers at boot; verify isolation with an intentional out-of-bounds access test in QEMU

**Technical notes:**
- ZSTs have zero runtime cost — `size_of::<UartCapability<0>>() == 0`
- Conflict detection at boot: build a `[bool; MAX_PERIPHERALS]` array, iterate task descriptors, assert each exclusive capability is claimed at most once
- This is O(N × P) where N = tasks, P = peripherals — both are small constants in the constrained target
- The allocator runs before the scheduler is started; if it halts, no task has ever run

---

### Sprint 4 — Scheduler (Weeks 9–10)

**Goal:** The CA-PIP scheduler is running. Priority inversion is demonstrably impossible. A test scenario shows a low-priority task inheriting priority correctly.

**Deliverables:**
- `CapabilityContentionGraph` built at boot from `TaskDescriptor` array
- `MaxInheritedPriority` computed for each task and stored in descriptor
- Preemptive fixed-priority scheduler running with round-robin tie-breaking
- Static priority queue: binary heap over `[TaskDescriptor; MAX_TASKS]`
- Demo scenario: 3 tasks (H, M, L) where L holds a capability H requires — demonstrate M cannot starve H via timing output on UART
- Scheduler overhead measured and logged (cycles per context switch)
- Cross-validate `MaxInheritedPriority` values against hand-computed SRP ceilings on the same task set (correctness test)
- **(Stretch — issue #36a)** Shift CCG construction from boot time into a Cargo build script; emit `MaxInheritedPriority` as a `const` array — eliminates boot latency and strips graph traversal logic from the final binary

**Technical notes:**
- CCG construction: for each pair (T_low, T_high) where T_low holds capability C and T_high requires C, add edge T_low → T_high
- MaxInheritedPriority(T) = max over all H reachable from T in CCG of priority(H); computed with simple BFS/DFS — this runs once at boot
- Priority queue: `heapq`-style over a fixed array; no `alloc`. `MAX_TASKS` is a const defined in `config.rs`
- Context switch latency target: ≤ 1000 cycles on RV32 at 10MHz → ≤ 100μs; measure with CLINT timer

---

### Sprint 5 — OML Integration (Weeks 11–12)

**Goal:** Task descriptors are defined in OML files, not hand-written Rust. `build.rs` drives the OML transpiler. Generated Rust compiles cleanly into the kernel binary.

**Deliverables:**
- `build.rs` invokes OML transpiler on `tasks/*.oml`, outputs to `src/generated/`
- OML schema defined for `Task { priority, stack_size, memory, peripheral, deadline }`
- All existing hand-written task descriptors from Sprint 3 replaced by OML-generated equivalents
- Build works without modification after adding a new `.oml` file
- Documentation: `docs/oml_schema.md` describing the OML dialect and generated output

**Technical notes:**
- `build.rs` uses `std::process::Command` to invoke the OML binary; path configured via env var or Cargo feature
- `println!("cargo:rerun-if-changed=tasks/")` ensures incremental rebuilds on OML file changes
- Generated Rust must be `#[allow(dead_code)]` and `#[automatically_derived]` annotated to avoid spurious warnings
- OML submodule pinned at a specific commit hash — no floating `main` dependency

---

### Sprint 6 — Polish & Demo (Weeks 13–14)

**Goal:** The project is presentable. The README is excellent. A demo is recorded. The binary fits the budget. Optionally: boots on physical hardware.

**Deliverables:**
- `README.md` complete: thesis, architecture diagram, build instructions, demo GIF
- `docs/ccg_algorithm.md`: detailed write-up of CCG construction and the priority inversion argument, including cross-validation against SRP
- `size_report.sh` output showing binary ≤ 512KB flash, static RAM ≤ 256KB
- Recorded QEMU demo showing 3-task scenario with UART output demonstrating CA-PIP behaviour
- All CI checks green
- GitHub releases: `v0.1.0` tagged with `ferret.elf` binary attached
- **(Stretch — issue #50a)** Integrate WCET analysis tooling (e.g. OTAWA for RISC-V); CI step fails if any task's computed WCET exceeds its declared OML deadline
- **(Stretch)** Boots and runs demo on ESP32-C3 or HiFive1 Rev B physical board

**Technical notes:**
- README architecture diagram: ASCII art or SVG; SVG preferred for GitHub rendering
- Demo recording: `asciinema` for terminal, or screen capture of QEMU window
- Release binary: built with `--release`, LTO enabled, `opt-level = "z"` for size
- Physical target (stretch): requires `probe-rs` for flashing, board-specific UART address adjustments

---

## GitHub Issues Breakdown

The following issues are ready to be created in the repository. Labels suggested: `kernel`, `scheduler`, `memory`, `capability`, `oml`, `tooling`, `docs`, `test`.

---

### Milestone 0: Foundations

| # | Title | Label | Assignee |
|---|---|---|---|
| 1 | Set up Cargo workspace with `riscv32imac-unknown-none-elf` target | `tooling` | |
| 2 | Write linker script `ferret.ld` for QEMU `virt` machine | `kernel` | |
| 3 | Implement minimal UART driver (memory-mapped 16550) | `kernel` | |
| 4 | Implement panic handler with UART output | `kernel` | |
| 5 | Boot to "Ferret booting..." in QEMU | `kernel` | |
| 6 | Write `run_qemu.sh` launch script | `tooling` | |
| 7 | Write `size_report.sh` binary size checker | `tooling` | |
| 8 | Set up GitHub Actions: build + QEMU boot check | `tooling` | |
| 9 | Set up GitHub Actions: Clippy linting | `tooling` | |
| 10 | Write README skeleton with thesis and architecture placeholder | `docs` | |

---

### Milestone 1: Kernel Core

| # | Title | Label | Assignee |
|---|---|---|---|
| 11 | Define RISC-V trap frame struct (32 GPRs + CSRs) | `kernel` | |
| 12 | Write `#[naked]` trap entry handler in assembly | `kernel` | |
| 13 | Implement CLINT timer interface (read `mtime`, write `mtimecmp`) | `kernel` | |
| 14 | Enable machine-mode timer interrupt (`mstatus`, `mie`, `mtvec`) | `kernel` | |
| 15 | Write context save/restore routine in `switch.S` | `kernel` | |
| 16 | Demonstrate single task interrupted and resumed correctly | `test` | |

---

### Milestone 2: Memory Safety Layer

| # | Title | Label | Assignee |
|---|---|---|---|
| 17 | Define `MemoryRegion<START, END>` with const generic bounds | `memory` | |
| 18 | Implement compile-time non-overlap check for `MemoryRegion` | `memory` | |
| 19 | Define `Stack<N>` with 16-byte alignment | `memory` | |
| 20 | Define `TaskDescriptor` struct with memory and stack fields | `kernel` | |
| 21 | Implement static `TaskRegistry`: fixed array of `TaskDescriptor` | `kernel` | |
| 22 | Verify total static allocation ≤ 256KB in CI via `size_report.sh` | `tooling` | |

---

### Milestone 3: Capability System

| # | Title | Label | Assignee |
|---|---|---|---|
| 23 | Define ZST capability types: `UartCapability<N>`, `GpioCapability<PIN>` | `capability` | |
| 24 | Define `ExclusiveCapability<T>` and `SharedCapability<T>` wrappers | `capability` | |
| 25 | Implement boot-time capability conflict detector | `capability` | |
| 26 | Halt with diagnostic on exclusive capability conflict | `capability` | |
| 27 | Write unit tests for capability conflict detection | `test` | |
| 28 | Add capability fields to `TaskDescriptor` | `capability` | |
| 28a | *(Stretch)* Map `MemoryRegion` types to RISC-V PMP registers at boot | `capability` | |

---

### Milestone 4: Scheduler

| # | Title | Label | Assignee |
|---|---|---|---|
| 29 | Define `TaskState` enum: `Ready`, `Running`, `Blocked`, `Suspended` | `scheduler` | |
| 30 | Implement static priority queue (`[TaskDescriptor; MAX_TASKS]` max-heap) | `scheduler` | |
| 31 | Implement CCG construction from `TaskRegistry` | `scheduler` | |
| 32 | Implement `MaxInheritedPriority` computation (BFS over CCG) | `scheduler` | |
| 33 | Integrate `MaxInheritedPriority` into scheduler preemption decision | `scheduler` | |
| 34 | Implement preemptive fixed-priority scheduler with round-robin tie-breaking | `scheduler` | |
| 35 | Write 3-task demo (H, M, L) demonstrating CA-PIP behaviour | `test` | |
| 36 | Measure and log context switch latency via CLINT | `scheduler` | |
| 36a | *(Stretch)* Shift CCG to Cargo build script; emit `MaxInheritedPriority` as `const` array | `scheduler` | |
| 36b | Cross-validate `MaxInheritedPriority` against hand-computed SRP ceilings | `test` | |

---

### Milestone 5: OML Integration

| # | Title | Label | Assignee |
|---|---|---|---|
| 37 | Define OML schema for `Task` (priority, stack_size, memory, peripheral, deadline) | `oml` | |
| 38 | Add OML as workspace submodule, pinned at commit | `oml` | |
| 39 | Write `build.rs` to invoke OML transpiler on `tasks/*.oml` | `oml` | |
| 40 | Define OML → Rust code generation template for `TaskDescriptor` | `oml` | |
| 41 | Replace hand-written `TaskDescriptor`s with OML-generated equivalents | `oml` | |
| 42 | Verify incremental rebuild works after adding new `.oml` file | `oml` | |
| 43 | Write `docs/oml_schema.md` | `docs` | |

---

### Milestone 6: Polish & Demo

| # | Title | Label | Assignee |
|---|---|---|---|
| 44 | Write complete `README.md` with thesis, build instructions, demo | `docs` | |
| 45 | Create architecture SVG diagram for README | `docs` | |
| 46 | Write `docs/ccg_algorithm.md` with full algorithm, SRP cross-validation, and invariant argument | `docs` | |
| 47 | Record QEMU demo showing CA-PIP 3-task scenario | `docs` | |
| 48 | Enable LTO and `opt-level = "z"` in release profile | `tooling` | |
| 49 | Tag `v0.1.0` release with `ferret.elf` binary | `tooling` | |
| 50 | *(Stretch)* Port to ESP32-C3 physical target | `kernel` | |
| 50a | *(Stretch)* Integrate WCET analysis tooling; CI fails if task WCET exceeds declared deadline | `tooling` | |

---

## Collaboration Guide

### Division of Labour (Suggested)

This project naturally splits into two tracks that can proceed in parallel after Sprint 1:

**Track A — Kernel & Scheduler (systems-heavy)**
Sprints 1–4: interrupt handling, memory model, capability system, scheduler implementation. Requires comfort with Rust `no_std`, assembly, and low-level hardware concepts.

**Track B — Tooling & OML (language/build-heavy)**
Sprints 0 and 5–6: build system, OML schema and transpiler integration, CI, documentation, demo. Requires comfort with Rust build scripts, the OML codebase, and documentation tooling.

Both collaborators should be involved in Sprint 0 (setup) and Sprint 6 (polish). The CCG algorithm design (Sprint 4, issues 31–33) is a good candidate for pair programming regardless of track assignment.

### Branching Convention

```
main          ← always builds, always passes CI
dev           ← integration branch, rebased from main
feat/<issue>  ← one branch per issue (e.g. feat/31-ccg-construction)
fix/<issue>   ← for bug fixes
docs/<issue>  ← documentation-only changes
```

### Commit Convention

```
feat(scheduler): implement CCG construction from TaskRegistry (#31)
fix(memory): correct stack alignment for 16-byte boundary (#19)
docs(readme): add architecture diagram and demo GIF (#45)
test(capability): add conflict detection unit tests (#27)
chore(ci): add QEMU smoke test to build workflow (#8)
```

### PR Policy

- Every PR targets `dev`, not `main`
- Every PR must pass CI (build + Clippy + QEMU boot check)
- At least one review required before merge
- PRs should reference the issue they close (`Closes #31`)
- Merge via squash commit to keep `dev` history clean

---

## Glossary

| Term | Definition |
|---|---|
| **CA-PIP** | Capability-Aware Priority Inheritance Protocol — Ferret's novel scheduler variant |
| **CCG** | Capability Contention Graph — directed graph of tasks and their resource dependencies, built at boot |
| **Capability** | A typed token representing access rights to a hardware resource; enforced at the type level |
| **CLINT** | Core Local Interruptor — RISC-V hardware block providing timer and software interrupts |
| **ICPP** | Immediate Ceiling Priority Protocol — a variant of PCP where a task's priority is bumped to the resource ceiling immediately upon acquisition, regardless of contention |
| **MaxInheritedPriority** | Precomputed value per task: the highest priority of any task that could be blocked by this task's resource holdings, computed via transitive closure over the CCG |
| **Microkernel** | An OS architecture where the kernel provides only minimal services (scheduling, IPC, memory); everything else is a task |
| **MMU** | Memory Management Unit — hardware for virtual memory; Ferret explicitly targets systems without one |
| **MCS** | Mixed Criticality Systems — an seL4 extension supporting tasks with different safety-criticality levels sharing a single CPU with formal timing guarantees |
| **OML** | Object Markup Language — the DSL used for task declarations; transpiles to Rust at build time |
| **PCP** | Priority Ceiling Protocol — assigns a static ceiling priority to each resource; prevents deadlocks and bounds blocking to one critical section |
| **PIP** | Priority Inheritance Protocol — a reactive scheduling mechanism where a low-priority task temporarily inherits the priority of a blocked high-priority task |
| **PMP** | Physical Memory Protection — a RISC-V hardware mechanism that partitions physical memory into regions with configurable access rights per CPU privilege mode |
| **Priority Inversion** | A scheduling anomaly where a high-priority task is blocked by a low-priority task; structurally impossible in Ferret |
| **RISC-V** | An open ISA; Ferret targets the 32-bit `RV32IMAC` variant |
| **SRP** | Stack Resource Policy — a proactive scheduling protocol used by the RTIC framework; computes static priority ceilings at compile time; mathematically related to CA-PIP |
| **TCB** | Trusted Computing Base — the minimal set of software and hardware components critical to a system's security; Ferret's TCB is the kernel core |
| **WCET** | Worst-Case Execution Time — a provable upper bound on a task's execution time for any input; amenable to static analysis in Ferret due to its fully static task model |
| **ZST** | Zero-Sized Type — a Rust type with `size_of() == 0`; used for capabilities so they have no runtime overhead |
