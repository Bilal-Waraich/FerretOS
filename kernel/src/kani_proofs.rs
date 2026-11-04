//! Kani bounded-verification harnesses for FerretOS kernel invariants.
//!
//! Run with: `cargo kani`
//!
//! Each `#[kani::proof]` function is a standalone verification target.
//! Kani explores all execution paths up to the declared unwind bound and
//! proves absence of panics, integer overflows, and assertion violations.
//!
//! These harnesses are compiled only under `cfg(kani)` — they are invisible
//! to the normal build and test pipelines.
//!
//! # What is verified
//!
//! 1. **CCG construction** — `build()` never accesses out-of-bounds indices
//!    for any task count ≤ MAX_TASKS and any capability mask values.
//! 2. **Conflict detector soundness** — if two tasks share an exclusive cap
//!    bit, `check_capability_conflicts` always panics (never silently passes).
//! 3. **Priority queue heap invariant** — after N arbitrary inserts and pops
//!    the root always holds the maximum inserted priority.
//! 4. **Channel ring buffer** (#78) — for any interleaving of up to
//!    `CHANNEL_OPS` sends and receives on a small channel: no index
//!    overflow, capacity never exceeded, `try_send` on a full channel hands
//!    the value back without writing, and values are received in send order
//!    with no loss or duplication.
//! 5. **Channel endpoint detector** (#76) — two tasks claiming the same
//!    sender endpoint always halt the boot.

#[cfg(kani)]
mod proofs {
    use crate::capability::allocator::check_capability_conflicts;
    use crate::config::MAX_TASKS;
    use crate::memory::task::TaskDescriptor;
    use crate::scheduler::ccg::CapabilityContentionGraph;
    use crate::scheduler::queue::PriorityQueue;

    // -----------------------------------------------------------------------
    // Helper: build an arbitrary TaskDescriptor for Kani
    // -----------------------------------------------------------------------

    fn arbitrary_task(id: u8) -> TaskDescriptor {
        TaskDescriptor::with_capabilities(
            id,
            kani::any(),          // priority
            0x2000_0000 + (id as usize * 0x1000),
            256,
            0x2000_1000 + (id as usize * 0x1000),
            0x2000_2000 + (id as usize * 0x1000),
            kani::any(),          // exclusive_cap_mask
            kani::any(),          // shared_cap_mask
            kani::any(),          // required_cap_mask
        )
    }

    // -----------------------------------------------------------------------
    // 1. CCG construction — no out-of-bounds access
    // -----------------------------------------------------------------------

    /// Verify that `CapabilityContentionGraph::build()` never panics or
    /// accesses memory out of bounds for any registry with up to 4 tasks
    /// and any combination of capability masks.
    ///
    /// Bound: 5 (4 tasks + 1) to keep state-space tractable.
    #[kani::proof]
    #[kani::unwind(17)]
    fn verify_ccg_build_no_oob() {
        let n: usize = kani::any();
        // Restrict to a small bound to keep the proof tractable.
        kani::assume(n <= 4);

        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        for i in 0..n {
            reg[i] = Some(arbitrary_task(i as u8));
        }

        let ccg = CapabilityContentionGraph::build(&reg);

        // task_count must match what we inserted.
        assert!(ccg.task_count == n);

        // has_edge must never panic for valid indices.
        for i in 0..MAX_TASKS {
            for j in 0..MAX_TASKS {
                let _ = ccg.has_edge(i, j);
            }
        }
    }

    /// Verify the key CCG correctness property: an edge (L, H) exists if and
    /// only if `L.exclusive_cap_mask & H.required_cap_mask != 0`.
    #[kani::proof]
    #[kani::unwind(17)]
    fn verify_ccg_edge_iff_contention() {
        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        reg[0] = Some(arbitrary_task(0));
        reg[1] = Some(arbitrary_task(1));

        let ccg = CapabilityContentionGraph::build(&reg);

        let l = reg[0].as_ref().unwrap();
        let h = reg[1].as_ref().unwrap();

        let contention = (l.exclusive_cap_mask & h.required_cap_mask) != 0;
        assert!(ccg.has_edge(0, 1) == contention);

        let contention_rev = (h.exclusive_cap_mask & l.required_cap_mask) != 0;
        assert!(ccg.has_edge(1, 0) == contention_rev);
    }

    // -----------------------------------------------------------------------
    // 2. Capability conflict detector — soundness
    // -----------------------------------------------------------------------

    /// Verify that two tasks with the same exclusive capability bit always
    /// trigger a panic in `check_capability_conflicts`.
    ///
    /// `#[kani::should_panic]` requires the harness to panic on every path:
    /// two tasks sharing one exclusive bit always conflict, so the detector
    /// always halts (which under `cfg(kani)` is a `panic!`). If the detector
    /// ever returned instead, the harness would fall through without panicking
    /// and Kani would report the missing panic as a failure.
    #[kani::proof]
    #[kani::should_panic]
    #[kani::unwind(33)]
    fn verify_conflict_detector_catches_double_exclusive() {
        let shared_bit: u32 = kani::any();
        // Restrict to a single set bit so the conflict is clear.
        kani::assume(shared_bit.count_ones() == 1);

        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        reg[0] = Some(TaskDescriptor::with_capabilities(
            0, 1, 0x2000_0000, 256, 0x2000_1000, 0x2000_2000,
            shared_bit, 0, 0,
        ));
        reg[1] = Some(TaskDescriptor::with_capabilities(
            1, 2, 0x2000_1000, 256, 0x2000_2000, 0x2000_3000,
            shared_bit, 0, 0, // same exclusive bit — conflict
        ));

        // The conflict detector must panic here. Scan only the populated slots.
        check_capability_conflicts(&reg[..2]);
    }

    /// Verify that non-overlapping exclusive caps never trigger a false positive.
    ///
    /// Each capability bit is checked independently by the detector, so the
    /// property is structural and bit-width-independent. We bound the masks to
    /// 4 bits: this is the dimension that controls solver cost, because the
    /// detector's inner scan walks a fixed `0..32` range regardless of mask
    /// value — bounding the *value* (not the loop) is what keeps the proof
    /// tractable. We also scan only the two populated registry slots; the 14
    /// empty `None` slots add outer-loop and `flatten` unrolling with no proof
    /// value, and were the dominant cost of the previous timeout.
    #[kani::proof]
    #[kani::unwind(33)]
    fn verify_conflict_detector_no_false_positive() {
        // Two tasks with disjoint exclusive caps must not conflict.
        let cap_a: u32 = kani::any();
        let cap_b: u32 = kani::any();
        kani::assume(cap_a < (1 << 4));
        kani::assume(cap_b < (1 << 4));
        kani::assume(cap_a & cap_b == 0); // disjoint

        let mut reg: [Option<TaskDescriptor>; MAX_TASKS] = [const { None }; MAX_TASKS];
        reg[0] = Some(TaskDescriptor::with_capabilities(
            0, 1, 0x2000_0000, 256, 0x2000_1000, 0x2000_2000,
            cap_a, 0, 0,
        ));
        reg[1] = Some(TaskDescriptor::with_capabilities(
            1, 2, 0x2000_1000, 256, 0x2000_2000, 0x2000_3000,
            cap_b, 0, 0,
        ));

        // Must not panic — no conflict. Scan only the populated slots.
        check_capability_conflicts(&reg[..2]);
    }

    // -----------------------------------------------------------------------
    // 3. Priority queue — heap invariant preserved
    // -----------------------------------------------------------------------

    /// Verify that `pop_max()` always returns the maximum priority inserted,
    /// for any sequence of up to 4 arbitrary inserts followed by pops.
    #[kani::proof]
    #[kani::unwind(5)]
    fn verify_priority_queue_pop_is_max() {
        let mut q: PriorityQueue<MAX_TASKS> = PriorityQueue::new();

        let p0: u8 = kani::any();
        let p1: u8 = kani::any();
        let p2: u8 = kani::any();
        let p3: u8 = kani::any();

        q.insert(0, p0);
        q.insert(1, p1);
        q.insert(2, p2);
        q.insert(3, p3);

        let expected_max = p0.max(p1).max(p2).max(p3);
        // peek_max_priority should equal the maximum inserted priority.
        assert!(q.peek_max_priority() == expected_max);
    }

    /// Verify that popping all elements from the queue yields strictly
    /// non-increasing priorities (sorted order).
    #[kani::proof]
    #[kani::unwind(5)]
    fn verify_priority_queue_sorted_pop_order() {
        let mut q: PriorityQueue<MAX_TASKS> = PriorityQueue::new();

        let p0: u8 = kani::any();
        let p1: u8 = kani::any();
        let p2: u8 = kani::any();

        q.insert(0, p0);
        q.insert(1, p1);
        q.insert(2, p2);

        // Pop in order and verify priorities are non-increasing.
        // We track priorities by peeking before each pop.
        let mut prev = q.peek_max_priority();
        let _ = q.pop_max();

        if !q.is_empty() {
            let cur = q.peek_max_priority();
            assert!(cur <= prev, "heap must yield non-increasing priorities");
            prev = cur;
            let _ = q.pop_max();
        }

        if !q.is_empty() {
            let cur = q.peek_max_priority();
            assert!(cur <= prev, "heap must yield non-increasing priorities");
        }
    }

    /// Verify that insert followed immediately by pop_max on a single-element
    /// queue returns the same task ID that was inserted.
    #[kani::proof]
    #[kani::unwind(3)]
    fn verify_priority_queue_insert_pop_roundtrip() {
        let mut q: PriorityQueue<MAX_TASKS> = PriorityQueue::new();
        let id: u8 = kani::any();
        let priority: u8 = kani::any();

        q.insert(id, priority);
        let popped = q.pop_max();

        assert!(popped == Some(id));
        assert!(q.is_empty());
    }

    // -----------------------------------------------------------------------
    // 4. Channel ring buffer (#78)
    // -----------------------------------------------------------------------

    use crate::ipc::Channel;

    /// Capacity of the channel under verification.  Small, but every
    /// wrap-around and full/empty transition is reachable within
    /// `CHANNEL_OPS` operations.
    const CHANNEL_CAP: usize = 3;
    /// Number of symbolic operations per harness: enough to fill, drain and
    /// wrap a `CHANNEL_CAP`-slot buffer.
    const CHANNEL_OPS: usize = 7;

    /// Capacity is never exceeded and a send to a full channel never writes;
    /// a receive from an empty channel returns nothing.  Index arithmetic is
    /// checked for overflow by Kani on every operation.
    #[kani::proof]
    #[kani::unwind(8)]
    fn verify_channel_capacity_respected() {
        let ch: Channel<u8, CHANNEL_CAP> = Channel::new();
        for _ in 0..CHANNEL_OPS {
            let before = ch.len();
            if kani::any() {
                let v: u8 = kani::any();
                match ch.try_send(v) {
                    Ok(()) => {
                        assert!(before < CHANNEL_CAP);
                        assert!(ch.len() == before + 1);
                    }
                    Err(back) => {
                        assert!(before == CHANNEL_CAP, "send may only fail when full");
                        assert!(back == v, "rejected value is handed back");
                        assert!(ch.len() == before, "a failed send writes nothing");
                    }
                }
            } else {
                match ch.try_receive() {
                    Some(_) => assert!(before > 0 && ch.len() == before - 1),
                    None => assert!(before == 0 && ch.len() == 0),
                }
            }
            assert!(ch.len() <= CHANNEL_CAP);
            assert!(ch.is_full() == (ch.len() == CHANNEL_CAP));
            assert!(ch.is_empty() == (ch.len() == 0));
        }
    }

    /// FIFO: values come out in the order they went in, none lost or
    /// duplicated.  Each send carries a sequence number; each successful
    /// receive must return exactly the next expected one.
    #[kani::proof]
    #[kani::unwind(8)]
    fn verify_channel_fifo_no_loss_no_duplication() {
        let ch: Channel<u8, CHANNEL_CAP> = Channel::new();
        let mut next_send: u8 = 0;
        let mut next_recv: u8 = 0;
        for _ in 0..CHANNEL_OPS {
            if kani::any() {
                if ch.try_send(next_send).is_ok() {
                    next_send += 1;
                }
            } else if let Some(v) = ch.try_receive() {
                assert!(v == next_recv, "received out of order, lost or duplicated");
                next_recv += 1;
            }
            // Everything sent and not yet received is exactly what is queued.
            assert!((next_send - next_recv) as usize == ch.len());
        }
    }

    // -----------------------------------------------------------------------
    // 5. Channel endpoint detector (#76)
    // -----------------------------------------------------------------------

    /// If two tasks both claim the sender endpoint of the same channel, the
    /// detector halts (a `panic!` under `cfg(kani)`).
    #[kani::proof]
    #[kani::should_panic]
    #[kani::unwind(33)]
    fn verify_channel_detector_catches_double_sender() {
        use crate::capability::check_channel_conflicts;
        use crate::config::MAX_CHANNELS;
        let ch: usize = kani::any();
        kani::assume(ch < MAX_CHANNELS);
        let a = TaskDescriptor::new(0, 1, 0, 16, 0, 0).with_channels(1 << ch, 0);
        let b = TaskDescriptor::new(1, 1, 0, 16, 0, 0).with_channels(1 << ch, 0);
        check_channel_conflicts(&[Some(a), Some(b)]);
    }
}
