import Mathlib

/-!
# FerretOS — Capability Contention Graph (CCG) and MaxInheritedPriority (MIP)

A formal model of the CA-PIP scheduler's core structures, mirroring the Rust
implementation in `kernel/src/scheduler/ccg.rs` and `kernel/src/scheduler/mip.rs`.

## Modelling choices

* **Priorities are `ℕ`, not `UInt8`.** The CA-PIP results are independent of the
  priority bit-width, and `ℕ` carries the `OrderBot` (`⊥ = 0`) and
  `SemilatticeSup` (`sup = max`) instances that `Finset.sup` and the
  priority-inversion proofs depend on. The kernel stores priorities as `u8`.
* **Capability masks stay `UInt32`.** The contention test is the bitwise
  `exclusive &&& required ≠ 0`, identical to the kernel, so the model's notion
  of an edge matches the implementation exactly.
* **Reachability includes the source.** `mip` sups over the reflexive-transitive
  closure, matching `compute_and_store_mip`, which seeds each task with its own
  base priority before the BFS.

This file corresponds to issue #66 (model + definitions). The priority-inversion
theorem (`mip ≥ srp_ceiling`) is issue #67 and builds on these definitions.
-/

namespace FerretOS

-- Classical decidability: `reachable` is the transitive closure of a relation
-- and is not given a decidable instance here, so the `if`-guards below elaborate
-- via `Classical.propDecidable`. The definitions are specifications, not code,
-- hence `noncomputable`.
open Classical

/-- Maximum number of tasks; mirrors `MAX_TASKS` in `kernel/src/config.rs`. -/
def MAX_TASKS : ℕ := 16

/-- A task index into the static registry. -/
abbrev TaskId := Fin MAX_TASKS

/-- Base scheduling priority. Modelled as `ℕ`; the kernel uses `u8`. -/
abbrev Priority := ℕ

/-- The scheduling-relevant fields of a task: base priority and capability masks.
    Mirrors the `priority` / `exclusive_cap_mask` / `required_cap_mask` fields of
    the Rust `TaskDescriptor`. -/
structure Task where
  priority       : Priority
  exclusive_caps : UInt32
  required_caps  : UInt32

variable {n : ℕ}

/-- CCG edge `l → h`: task `l` holds an exclusive capability that task `h`
    requires. Mirrors `l.exclusive_cap_mask & h.required_cap_mask != 0` in
    `ccg.rs`, including the `l ≠ h` guard from the construction loop. -/
def ccg_edge (tasks : Fin n → Task) (l h : Fin n) : Prop :=
  l ≠ h ∧ (tasks l).exclusive_caps &&& (tasks h).required_caps ≠ 0

/-- Reachability over the CCG: the reflexive-transitive closure of `ccg_edge`.
    Mirrors the BFS in `mip.rs`, which reaches the source itself. -/
def reachable (tasks : Fin n → Task) (src dst : Fin n) : Prop :=
  Relation.ReflTransGen (ccg_edge tasks) src dst

/-- `MaxInheritedPriority(t)`: the greatest base priority among all tasks
    reachable from `t` (including `t`). Mirrors `compute_and_store_mip`. -/
noncomputable def mip (tasks : Fin n → Task) (t : Fin n) : Priority :=
  Finset.univ.sup fun u => if reachable tasks t u then (tasks u).priority else 0

/-- The SRP-style ceiling of capability bit-set `c`: the greatest base priority
    among all tasks that require any bit of `c`. This is the quantity CA-PIP must
    dominate for priority inversion to be impossible. -/
noncomputable def srp_ceiling (tasks : Fin n → Task) (c : UInt32) : Priority :=
  Finset.univ.sup fun u => if (tasks u).required_caps &&& c ≠ 0 then (tasks u).priority else 0

/-- Effective scheduling priority: `max(base priority, MIP)`. Mirrors
    `TaskDescriptor::effective_priority` in `kernel/src/memory/task.rs`. This is
    the value the runtime scheduler actually compares. -/
noncomputable def effective_priority (tasks : Fin n → Task) (t : Fin n) : Priority :=
  max (tasks t).priority (mip tasks t)

-- Acceptance criteria for issue #66: these elaborate against Lean 4 + Mathlib.
#check @mip
#check @srp_ceiling

end FerretOS
