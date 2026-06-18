import FerretOS.CCG

/-!
# FerretOS — CA-PIP correctness: priority inversion is structurally impossible

This file (issue #67) proves the central scheduling guarantee on the model from
`FerretOS.CCG`: a task that holds a capability another task needs always has an
effective priority at least that of the waiter, so it can never be preempted by
a task waiting on it.

## A note on the statement

Issue #67 originally phrased the theorem against `srp_ceiling` for an arbitrary
capability bit-set `c`. Formalising it surfaced that this is **false for a
multi-bit `c`**: "the holder owns *some* bit of `c`" and "the waiter needs *some*
bit of `c`" do not imply they share a bit, so there need be no CCG edge between
them. The faithful, stronger guarantee is stated directly in terms of a CCG edge
(`ccg_edge holder waiter`), which is precisely "the holder owns something the
waiter requires". The `srp_ceiling` form is recoverable as a corollary under a
single-bit hypothesis on `c`; that bitmask lemma is left as future work.
-/

namespace FerretOS

open Classical

variable {n : ℕ}

/-- Every task reachable from `t` over the CCG contributes its base priority to
    `MIP(t)`. This is the workhorse: `mip` is a supremum over all reachable
    tasks, so any single reachable task's priority is a lower bound. -/
theorem mip_ge_of_reachable (tasks : Fin n → Task) {t u : Fin n}
    (h : reachable tasks t u) :
    (tasks u).priority ≤ mip tasks t := by
  have hle : (fun v => if reachable tasks t v then (tasks v).priority else 0) u
      ≤ mip tasks t :=
    Finset.le_sup (Finset.mem_univ u)
  simpa [if_pos h] using hle

/-- A task's own base priority never exceeds its MIP (reachability is reflexive,
    so the task itself is in the supremum). Mirrors the `mip ≥ base_priority`
    seeding in `compute_and_store_mip`. -/
theorem priority_le_mip (tasks : Fin n → Task) (t : Fin n) :
    (tasks t).priority ≤ mip tasks t :=
  mip_ge_of_reachable tasks Relation.ReflTransGen.refl

/-- **No priority inversion (MIP form).** If `holder` owns a capability that
    `waiter` requires (a direct CCG edge), then `MIP(holder) ≥ priority(waiter)`. -/
theorem no_priority_inversion_mip (tasks : Fin n → Task) {holder waiter : Fin n}
    (h : ccg_edge tasks holder waiter) :
    (tasks waiter).priority ≤ mip tasks holder :=
  mip_ge_of_reachable tasks (Relation.ReflTransGen.single h)

/-- **No priority inversion (effective-priority form).** The holder's *effective*
    priority — what the scheduler actually compares — is at least the waiter's
    base priority. Hence a waiter can never preempt the task it is waiting on:
    priority inversion is impossible by construction, not corrected reactively. -/
theorem no_priority_inversion (tasks : Fin n → Task) {holder waiter : Fin n}
    (h : ccg_edge tasks holder waiter) :
    (tasks waiter).priority ≤ effective_priority tasks holder := by
  unfold effective_priority
  exact le_trans (no_priority_inversion_mip tasks h) (le_max_right _ _)

end FerretOS
