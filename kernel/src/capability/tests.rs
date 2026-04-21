//! Host-side unit tests for capability conflict detection.
//!
//! Run with `cargo test` (host target, not riscv32imac).

#[cfg(test)]
mod tests {
    use crate::capability::allocator::check_capability_conflicts;
    use crate::memory::task::TaskDescriptor;

    /// Build a minimal descriptor with explicit capability masks.
    fn make_task(id: u8, exclusive: u32, shared: u32) -> TaskDescriptor {
        TaskDescriptor::with_capabilities(
            id,
            1,          // priority
            0x2000_0000, // stack_base (dummy)
            256,         // stack_size
            0x2000_0000, // memory_start (dummy)
            0x2000_1000, // memory_end   (dummy)
            exclusive,
            shared,
            0,           // required_cap_mask
        )
    }

    #[test]
    fn test_no_conflict_passes() {
        // Task 0 holds peripheral 0 exclusively, task 1 holds peripheral 1.
        let registry: [Option<TaskDescriptor>; 2] = [
            Some(make_task(0, 0b01, 0)),
            Some(make_task(1, 0b10, 0)),
        ];
        // Must not panic.
        check_capability_conflicts(&registry);
    }

    #[test]
    #[should_panic]
    fn test_exclusive_conflict_halts() {
        // Both tasks claim peripheral 0 exclusively — must halt (panic in test).
        let registry: [Option<TaskDescriptor>; 2] = [
            Some(make_task(0, 0b1, 0)),
            Some(make_task(1, 0b1, 0)),
        ];
        check_capability_conflicts(&registry);
    }

    #[test]
    fn test_shared_capability_allowed() {
        // Both tasks hold peripheral 0 as shared — no conflict.
        let registry: [Option<TaskDescriptor>; 2] = [
            Some(make_task(0, 0, 0b1)),
            Some(make_task(1, 0, 0b1)),
        ];
        check_capability_conflicts(&registry);
    }

    #[test]
    fn test_multiple_exclusive_same_task_ok() {
        // One task holds UART0 (bit 0) and GPIO13 (bit 13) exclusively.
        let registry: [Option<TaskDescriptor>; 1] = [
            Some(make_task(0, (1 << 0) | (1 << 13), 0)),
        ];
        check_capability_conflicts(&registry);
    }

    #[test]
    fn test_empty_registry_passes() {
        let registry: [Option<TaskDescriptor>; 4] = [None, None, None, None];
        check_capability_conflicts(&registry);
    }

    #[test]
    fn test_none_slots_ignored() {
        // Only slot 0 populated; slots 1-3 are None.
        let registry: [Option<TaskDescriptor>; 4] = [
            Some(make_task(0, 0b111, 0)),
            None,
            None,
            None,
        ];
        check_capability_conflicts(&registry);
    }
}
