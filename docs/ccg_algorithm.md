# CCG Construction and CA-PIP Algorithm

This document describes the Capability Contention Graph (CCG) construction algorithm, the MaxInheritedPriority (MIP) computation, and the CA-PIP preemption rule. It also cross-validates CA-PIP against SRP and states the priority inversion impossibility invariant.

---

## 1. Definitions

**Task registry** — a static array of `N` `TaskDescriptor` entries populated at boot before any task runs. Each entry carries:

| Field | Symbol | Meaning |
|-------|--------|---------|
| `priority` | π(T) | Base scheduling priority (higher = more urgent) |
| `exclusive_cap_mask` | excl(T) | Bitmask of peripherals held exclusively by T |
| `required_cap_mask` | req(T) | Bitmask of peripherals T needs but does not yet hold |
| `max_inherited_priority` | MIP(T) | Computed at boot; highest priority reachable from T via CCG |

**Effective priority** — `eff(T) = max(π(T), MIP(T))`.

**Peripheral** — a hardware resource identified by a single bit in a `u32` bitmask. Bit `i` represents peripheral ID `i`. The kernel supports up to 32 peripherals (`MAX_PERIPHERALS = 32`).

---

## 2. CCG Construction

The Capability Contention Graph is a directed graph `G = (V, E)` where:

- `V` = the set of all registered tasks
- `(L, H) ∈ E` iff `excl(L) & req(H) != 0`

In words: there is an edge from L to H when L holds at least one peripheral that H requires. L is the *holder*, H is the *waiter*.

### Algorithm (O(N²) time, O(N²) space for the adjacency matrix)

```
for each task L in registry:
    for each task H in registry:
        if L != H and (excl(L) & req(H)) != 0:
            add edge (L, H) to CCG
```

**Implementation** — `kernel/src/scheduler/ccg.rs`, `CapabilityContentionGraph::build()`.

The adjacency matrix is a `[[bool; MAX_TASKS]; MAX_TASKS]` array allocated statically. For `MAX_TASKS = 16` this is 256 bytes — well within the RAM budget.

### Example (3-task demo)

| Task | excl | req |
|------|------|-----|
| L (id=0) | 0x1 (UART0) | 0x0 |
| M (id=1) | 0x0 | 0x0 |
| H (id=2) | 0x0 | 0x1 (UART0) |

Edges: `excl(L) & req(H) = 0x1 & 0x1 = 0x1 ≠ 0` → edge (L, H).

No other edges: `excl(M) = 0`, `req(M) = 0`, `req(L) = 0`.

```
CCG:  L ──→ H
      M (isolated)
```

---

## 3. MaxInheritedPriority Computation

For each task T, MIP(T) is the maximum base priority among all tasks reachable from T via a directed BFS over the CCG (excluding T itself).

### Algorithm (O(N²) per task, O(N³) total)

```
for each task T in registry:
    visited = {T}
    queue = [T]
    max_pri = 0
    while queue non-empty:
        node = dequeue(queue)
        for each successor S of node in CCG:
            if S not in visited:
                visited.add(S)
                max_pri = max(max_pri, π(S))
                enqueue(queue, S)
    T.max_inherited_priority = max_pri
```

**Implementation** — `kernel/src/scheduler/mip.rs`, `compute_and_store_mip()`.

The BFS queue is a fixed `[Option<u8>; MAX_TASKS]` array; no heap allocation required.

### Example (continued)

```
MIP(L): BFS from L → visits H → max(π(H)) = max(3) = 3
MIP(M): BFS from M → no successors → 0
MIP(H): BFS from H → no successors → 0
```

Effective priorities:

```
eff(L) = max(1, 3) = 3
eff(M) = max(2, 0) = 2
eff(H) = max(3, 0) = 3
```

---

## 4. CA-PIP Preemption Rule

The scheduler's preemption decision at each timer tick:

```
preempt(current, candidate) iff eff(candidate) > eff(current)
```

Both `eff` values are stored in `TaskDescriptor.max_inherited_priority` (with `effective_priority()` computing the max inline). The comparison is two integer comparisons with no graph traversal at runtime.

**Timer ISR flow** (`kernel/src/scheduler/mod.rs`, `tick()`):

1. Timer fires every `TICK_CYCLES` (1 ms at 10 MHz).
2. ISR calls `scheduler::tick()`.
3. `tick()` finds the highest `eff` task in the ready queue.
4. If `eff(best) > eff(current)`, context switch to `best`.
5. Timer rearmed; `mret` returns to the new task (or resumes current).

---

## 5. Priority Inversion Impossibility Invariant

**Invariant:** For every task T that holds an exclusive peripheral C, and every task W that requires C:

```
eff(T) ≥ π(W)
```

**Proof sketch:**

1. By CCG construction, `excl(T) & req(W) != 0` implies edge (T, W) ∈ E.
2. By MIP computation, `MIP(T) ≥ π(W)` (W is reachable from T via the direct edge).
3. By definition, `eff(T) = max(π(T), MIP(T)) ≥ MIP(T) ≥ π(W)`. □

**Consequence:** T cannot be preempted by W while T holds C, because `eff(T) ≥ π(W)` means W never has strictly higher effective priority than T. Priority inversion — a lower-priority task blocking a higher-priority waiter indefinitely — is structurally impossible.

For a multi-hop chain (T holds C₁; M holds C₂ that W₂ requires; W₂ requires C₁), the BFS propagates MIP transitively, extending the invariant to all reachable waiters.

---

## 6. CA-PIP vs. SRP Cross-Validation

SRP (Stack Resource Policy, used by RTIC) assigns each resource a *ceiling* equal to the maximum priority of any task that uses it. A task is blocked from preempting a critical section if the task's priority does not exceed the system ceiling.

**Equivalence condition:** When the resource graph has no multi-hop chains (each task requires at most one peripheral, no peripheral is required transitively), CA-PIP and SRP assign identical ceilings.

For the 3-task demo:

| Resource | SRP ceiling | CA-PIP MIP source |
|----------|-------------|-------------------|
| UART0 | max(π(L), π(H)) = max(1,3) = 3 | MIP(L) = π(H) = 3 |

Both approaches raise L's priority to 3. The test in `kernel/src/scheduler/tests.rs` (`test_mip_matches_srp_ceiling`) asserts this numerically for the demo configuration.

**Where CA-PIP differs:** For multi-hop chains — e.g., L holds C₁; M holds C₂ that H requires; H also requires C₁ — SRP per-resource ceilings may undercount unless the ceiling is taken over the full transitive closure. CA-PIP's BFS over the CCG handles transitivity automatically.

---

## 7. Complexity Summary

| Step | Time | Space |
|------|------|-------|
| CCG construction | O(N²) | O(N²) — adjacency matrix |
| MIP computation | O(N³) | O(N) — BFS visited set |
| Preemption check (runtime) | O(1) | — |

For `MAX_TASKS = 16`: CCG construction is 256 comparisons; MIP is at most 4096 BFS steps. Both complete in microseconds on any real hardware. All structures are statically allocated.
