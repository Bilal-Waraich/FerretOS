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

/// Start of the flash image (code + read-only data) — `kernel/memory.x`.
pub const FLASH_ORIGIN: usize = 0x8000_0000;

/// Size of the flash region in bytes (512 KB budget).
pub const FLASH_SIZE: usize = 512 * 1024;

/// Start of the task RAM area holding every task's memory region.
pub const TASK_RAM_ORIGIN: usize = 0x800B_8000;

/// Size of the task RAM area in bytes.
pub const TASK_RAM_SIZE: usize = 32 * 1024;
