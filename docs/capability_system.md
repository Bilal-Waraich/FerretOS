# Capability System

FerretOS models hardware peripherals as zero-sized types (ZSTs). This document covers the compile-time enforcement layer, the runtime bitmask bridge, and the boot-time conflict detector.

---

## 1. Definitions

**Peripheral** — a hardware resource (UART, GPIO, SPI, …) represented at the type level by a ZST and at runtime by a single bit in a `u32` bitmask. Bit `i` identifies peripheral ID `i`. The kernel supports up to `MAX_PERIPHERALS = 32` peripherals.

**Exclusive capability** — ownership that grants sole access. Only one task may hold `ExclusiveCapability<T>` for any given `T` at a time. Enforced at compile time via Rust's move semantics (non-`Clone`) and at boot time by the conflict detector.

**Shared capability** — read access that may be granted to multiple tasks simultaneously. `SharedCapability<T>` is `Clone`. Shared access does not create CCG edges.

---

## 2. ZST Hardware Types

Each supported peripheral has a zero-sized marker type in `capability/types.rs`:

```rust
pub struct UartCapability<const N: usize>;
pub struct GpioCapability<const PIN: usize>;
pub struct SpiCapability<const N: usize>;
pub struct I2cCapability<const N: usize>;
```

`size_of::<UartCapability<0>>() == 0`. No memory is allocated for these types. Their purpose is to carry peripheral identity into the type system so that ownership can be reasoned about at compile time.

---

## 3. Ownership Wrappers

```rust
pub struct ExclusiveCapability<T> { _phantom: PhantomData<T> }
pub struct SharedCapability<T>    { _phantom: PhantomData<T> }
```

| Wrapper | Clone | Copy | Meaning |
|---------|-------|------|---------|
| `ExclusiveCapability<T>` | No | No | Sole owner; cannot duplicate |
| `SharedCapability<T>` | Yes | Yes | Read-only; freely duplicatable |

Because `ExclusiveCapability<T>` is neither `Clone` nor `Copy`, Rust's move semantics make it structurally impossible to give the same exclusive capability to two tasks. This is a compile-time guarantee with zero runtime cost.

---

## 4. Bitmask Bridge

The type-level model must be connected to the runtime structures that the CCG builder and scheduler consume. Each `TaskDescriptor` carries three `u32` bitmask fields:

| Field | Meaning |
|-------|---------|
| `exclusive_cap_mask` | Bit `i` set ↔ task holds `ExclusiveCapability` of peripheral `i` |
| `shared_cap_mask` | Bit `i` set ↔ task holds `SharedCapability` of peripheral `i` |
| `required_cap_mask` | Bit `i` set ↔ task needs peripheral `i` but does not yet hold it |

The bitmask values are declared in the OML task definition (`tasks/demo_tasks.oml`) and compiled into static `TaskConfig` instances. `TaskConfig::into_descriptor()` copies the masks into `TaskDescriptor` at boot.

### Peripheral ID encoding

```
Bit 0 — UART0   (0x0000_0001)
Bit 1 — UART1   (0x0000_0002)
Bit 2 — GPIO0   (0x0000_0004)
Bit 3 — GPIO1   (0x0000_0008)
Bit 4 — SPI0    (0x0000_0010)
Bit 5 — I2C0    (0x0000_0020)
```

One-hot encoding allows O(1) conflict detection via bitwise AND.

---

## 5. Boot-Time Conflict Detector

`capability/allocator.rs`, `check_capability_conflicts()`.

### Algorithm (O(N × P) = O(1) for this target)

```
claimed = [None; MAX_PERIPHERALS]   // claimed[i] = task_id that holds peripheral i

for each task T in registry:
    for each bit i set in T.exclusive_cap_mask:
        if i >= MAX_PERIPHERALS:
            halt("peripheral ID out of range")
        if claimed[i] == Some(other_id):
            halt("peripheral i claimed by both other_id and T.id")
        claimed[i] = Some(T.id)
```

If a conflict is detected the kernel prints a UART diagnostic and spins forever — task execution never begins with a violated capability invariant.

### What the detector catches vs. what it does not

| Scenario | Detected? |
|----------|-----------|
| Two tasks both set the same bit in `exclusive_cap_mask` | ✅ Yes — halts at boot |
| One task sets a bit in `exclusive_cap_mask`, another sets the same bit in `required_cap_mask` | ❌ No — this is the valid holder–waiter pattern that creates CCG edges |
| Two tasks share the same bit in `shared_cap_mask` | ❌ No — shared access is explicitly allowed |

Separating `exclusive_cap_mask` from `required_cap_mask` is the key design decision: it prevents the conflict detector from falsely flagging the holder–waiter relationship that CA-PIP depends on.

---

## 6. Relationship to the CCG

After the conflict detector passes, the scheduler constructs the Capability Contention Graph:

```
edge (L, H)  iff  L.exclusive_cap_mask & H.required_cap_mask != 0
```

- Only `exclusive_cap_mask` contributes to CCG edges (shared access is non-blocking).
- The conflict detector guarantees that at most one task sets any given `exclusive_cap_mask` bit, so edges are always from a unique holder to one or more waiters — never from two competing holders.

For the full CCG construction and MIP algorithm, see [ccg_algorithm.md](ccg_algorithm.md).

---

## 7. Example: 3-Task Demo

| Task | `exclusive_cap_mask` | `required_cap_mask` | Role |
|------|---------------------|---------------------|------|
| L (id=0) | `0x1` (UART0) | `0x0` | Holds UART0 |
| M (id=1) | `0x0` | `0x0` | No contention |
| H (id=2) | `0x0` | `0x1` (UART0) | Waits for UART0 |

Conflict check: only L sets bit 0 in `exclusive_cap_mask` → no conflict → passes.

CCG edge: `L.exclusive (0x1) & H.required (0x1) = 0x1 ≠ 0` → edge L → H.

MIP(L) = π(H) = 3 → `eff_pri(L) = max(1, 3) = 3` → L cannot be preempted by M (pri 2).

---

## 8. Complexity Summary

| Step | Time | Space |
|------|------|-------|
| Conflict detection | O(N × P) | O(P) — `claimed` array |
| CCG construction | O(N²) | O(N²) — adjacency matrix |
| Boot total | O(N²) | O(N²) |

For `MAX_TASKS = 16` and `MAX_PERIPHERALS = 32`: conflict detection is 512 bit checks; CCG construction is 256 AND operations. Both complete in microseconds on any real hardware.
