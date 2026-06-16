# Changelog

## v0.1.0 — 2026-06-16

Initial public release. FerretOS is a `no_std` Rust microkernel for RISC-V systems with ≤256 KB RAM and no MMU, demonstrating the CA-PIP priority inversion prevention algorithm.

### Kernel

- RISC-V RV32IMAC bare-metal boot via `riscv-rt`; 16550 UART driver; CLINT timer at 1 ms tick
- Machine-mode interrupt handling with full 32-GPR trap frame save/restore
- Static task registry (`MAX_TASKS = 16`) with `TaskDescriptor` carrying base priority, capability masks, and precomputed `MaxInheritedPriority`
- `MemoryRegion<START, END>`: zero-sized type with compile-time non-overlap check
- `Stack<N>`: 16-byte aligned, statically allocated, RISC-V ABI compliant
- Boot-time capability conflict detector: halts with UART diagnostic if two tasks claim the same exclusive peripheral

### CA-PIP Scheduler

- `CapabilityContentionGraph::build()`: O(N²) construction from `exclusive_cap_mask` / `required_cap_mask` pairs
- `compute_and_store_mip()`: BFS from each task over the CCG, O(N³) total; writes `max_inherited_priority` to each `TaskDescriptor`
- `effective_priority(T) = max(base_priority, max_inherited_priority)` — constant for system lifetime
- Priority inversion structurally impossible: holder's effective priority ≥ every waiter's base priority by construction
- Preemption check is two integer comparisons; no graph traversal at runtime
- Static max-heap priority queue (`O(log N)` insert/pop)
- 3-task demo (L=1, M=2, H=3): `MIP(L) = 3`, `eff(L) = 3`; M cannot delay H via L

### OML Integration (Sprint 5)

- Task schema and instance declarations in `tasks/task.oml` and `tasks/demo_tasks.oml`
- `build.rs` invokes OML transpiler to generate `src/generated/task_schema.rs` and `src/generated/demo_tasks.rs`
- Generated files committed — kernel builds without OML installed
- `TaskConfig::into_descriptor()` bridge method in `src/generated/bridge.rs`
- OML pinned as a git submodule at `oml/`

### Documentation

- `FERRET.md` — full design rationale, algorithm proofs, ecosystem positioning
- `docs/oml_schema.md` — OML task schema field reference, peripheral bitmask encoding, build-time regeneration
- `docs/ccg_algorithm.md` — CCG construction, MIP BFS, CA-PIP preemption rule, priority inversion invariant proof, CA-PIP vs SRP cross-validation
- `docs/figures/architecture.svg` — three-layer architecture diagram

### CI

- GitHub Actions: build + QEMU boot integration test + size budget check (`build.yml`)
- GitHub Actions: Clippy zero-warning policy (`clippy.yml`)
- OML submodule checked out in both workflows
