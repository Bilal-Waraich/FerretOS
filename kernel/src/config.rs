//! Kernel-wide compile-time configuration constants.
//!
//! All statically-sized arrays — the task registry, priority queue, and
//! capability bitmask — are bounded by these constants.  Update them before
//! adding tasks or peripherals beyond the current limits.

/// Maximum number of tasks that can be registered at boot.
///
/// Bounds `TASK_REGISTRY` in `memory::task`.  Increasing this value grows
/// the registry's `.bss` footprint by `size_of::<Option<TaskDescriptor>>()`
/// per slot.
pub const MAX_TASKS: usize = 16;

/// Maximum number of peripherals tracked by the capability system.
///
/// Each `TaskDescriptor` carries a `held_capabilities: u32` bitmask (one bit
/// per peripheral).  This constant must not exceed 32.
pub const MAX_PERIPHERALS: usize = 32;

const _: () = assert!(
    MAX_PERIPHERALS <= 32,
    "MAX_PERIPHERALS must fit in a u32 bitmask (max 32)"
);

/// Maximum number of IPC channels addressable by capability ID.
///
/// Bounds the channel-endpoint conflict check in the capability allocator and
/// any static channel registry.  Channel IDs run `0..MAX_CHANNELS`.
pub const MAX_CHANNELS: usize = 8;
