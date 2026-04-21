//! Task descriptor and static task registry.
//!
//! `TaskDescriptor` carries the identity, scheduling, and memory layout of a
//! single kernel task.  `TASK_REGISTRY` is a fixed-size array of descriptors
//! populated at boot before the scheduler starts; after that point it is
//! treated as immutable.

use crate::config::MAX_TASKS;

/// Execution state of a task.
///
/// Transitions are managed by the scheduler (Sprint 4).  During Sprint 2 and
/// 3 all registered tasks are left in `Ready` after boot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    /// Task is eligible to run and waiting for the CPU.
    Ready,
    /// Task is currently executing on the CPU.
    Running,
    /// Task is waiting for a resource or event.
    Blocked,
    /// Task has been explicitly suspended and will not be scheduled.
    Suspended,
}

/// Describes a single task: identity, scheduling priority, and memory layout.
///
/// `#[repr(C)]` is required for predictable field offsets when the Sprint 4
/// scheduler's assembly stubs access this struct by fixed byte offsets.
///
/// # Memory model
///
/// All fields are plain values — no pointers into heap-allocated data.
/// `stack_ptr` is a raw address that will be installed as the task's `sp`
/// on first dispatch; it starts at `stack_base + stack_size` (top of stack).
///
/// # Capability extension (Sprint 3)
///
/// A `held_capabilities: u32` bitmask field will be added in Sprint 3.  The
/// `#[repr(C)]` layout means the offset of existing fields will not change.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskDescriptor {
    /// Unique task identifier.  Assigned by the caller of [`register_task`].
    pub id: u8,

    /// Static base priority (higher value = higher priority).
    ///
    /// The CA-PIP scheduler (Sprint 4) computes `effective_priority` as
    /// `max(priority, MaxInheritedPriority)` over the CCG.
    pub priority: u8,

    /// Current execution state.
    pub state: TaskState,

    /// Current stack pointer address.
    ///
    /// Initialised to `stack_base + stack_size` (top of a downward-growing
    /// stack).  Updated on every context switch.
    pub stack_ptr: usize,

    /// Address of the first byte of the stack buffer.
    pub stack_base: usize,

    /// Stack size in bytes.  Must be a multiple of 16 (RISC-V ABI).
    pub stack_size: usize,

    /// Start address of the task's private memory region (inclusive).
    pub memory_start: usize,

    /// End address of the task's private memory region (exclusive).
    pub memory_end: usize,

    /// Bitmask of exclusive peripheral IDs held by this task.
    ///
    /// Bit `i` set means this task holds exclusive ownership of peripheral `i`.
    /// The boot-time conflict detector (Sprint 3) checks that no two tasks share
    /// the same set bit.  The CCG builder (Sprint 4) reads this mask to create
    /// contention edges.
    pub exclusive_cap_mask: u32,

    /// Bitmask of shared peripheral IDs held by this task.
    ///
    /// Bit `i` set means this task holds shared (read-only) access to
    /// peripheral `i`.  Shared access does not create CCG edges.
    pub shared_cap_mask: u32,

    /// Bitmask of peripheral IDs this task requires but does not yet hold.
    ///
    /// Used by the CCG builder (Sprint 4) to construct contention edges:
    /// an edge L → H is added when `L.exclusive_cap_mask & H.required_cap_mask != 0`,
    /// meaning L holds a peripheral that H needs.  This is separate from
    /// `exclusive_cap_mask` so the boot-time conflict detector does not
    /// falsely flag a holder–waiter pair as a double-claim.
    pub required_cap_mask: u32,

    /// Maximum priority inherited via the CCG (computed once at boot).
    ///
    /// Set by [`crate::scheduler::compute_max_inherited_priorities`] to the
    /// highest base priority among all tasks reachable from this task via BFS
    /// over the CCG.  `effective_priority = max(priority, max_inherited_priority)`.
    pub max_inherited_priority: u8,
}

impl TaskDescriptor {
    /// Construct a new descriptor with no capabilities held.
    ///
    /// `stack_ptr` is initialised to `stack_base + stack_size` (top of
    /// the stack), which is the correct initial value for a RISC-V task
    /// that has not yet run.
    pub const fn new(
        id: u8,
        priority: u8,
        stack_base: usize,
        stack_size: usize,
        memory_start: usize,
        memory_end: usize,
    ) -> Self {
        TaskDescriptor {
            id,
            priority,
            state: TaskState::Ready,
            stack_ptr: stack_base + stack_size,
            stack_base,
            stack_size,
            memory_start,
            memory_end,
            exclusive_cap_mask: 0,
            shared_cap_mask: 0,
            required_cap_mask: 0,
            max_inherited_priority: 0,
        }
    }

    /// Construct a descriptor with explicit capability masks.
    // Nine arguments exceeds clippy's default limit (7) but a const fn cannot
    // use a builder pattern, and splitting the signature would obscure the
    // atomic nature of task construction.
    #[allow(clippy::too_many_arguments)]
    pub const fn with_capabilities(
        id: u8,
        priority: u8,
        stack_base: usize,
        stack_size: usize,
        memory_start: usize,
        memory_end: usize,
        exclusive_cap_mask: u32,
        shared_cap_mask: u32,
        required_cap_mask: u32,
    ) -> Self {
        TaskDescriptor {
            id,
            priority,
            state: TaskState::Ready,
            stack_ptr: stack_base + stack_size,
            stack_base,
            stack_size,
            memory_start,
            memory_end,
            exclusive_cap_mask,
            shared_cap_mask,
            required_cap_mask,
            max_inherited_priority: 0,
        }
    }

    /// Iterate over exclusive peripheral IDs held by this task.
    ///
    /// Yields each bit index `i` where `exclusive_cap_mask & (1 << i) != 0`.
    /// Time complexity: O(MAX_PERIPHERALS) = O(32).
    pub fn exclusive_capabilities(&self) -> impl Iterator<Item = usize> + '_ {
        (0..32).filter(move |&i| self.exclusive_cap_mask & (1 << i) != 0)
    }

    /// Iterate over shared peripheral IDs held by this task.
    pub fn shared_capabilities(&self) -> impl Iterator<Item = usize> + '_ {
        (0..32).filter(move |&i| self.shared_cap_mask & (1 << i) != 0)
    }

    /// Iterate over peripheral IDs required (but not yet held) by this task.
    pub fn required_capabilities(&self) -> impl Iterator<Item = usize> + '_ {
        (0..32).filter(move |&i| self.required_cap_mask & (1 << i) != 0)
    }

    /// Effective priority for CA-PIP scheduling.
    ///
    /// Returns `max(priority, max_inherited_priority)`.  Both fields are `u8`
    /// so the comparison is a single instruction; no heap allocation required.
    pub fn effective_priority(&self) -> u8 {
        self.priority.max(self.max_inherited_priority)
    }
}

// ---------------------------------------------------------------------------
// Static task registry
// ---------------------------------------------------------------------------

/// Global task registry — populated at boot, effectively immutable thereafter.
///
/// Declared `static mut` because it is written once during boot (before any
/// task runs or interrupt fires) and then read by the scheduler.  The
/// single-hart, cooperative-boot invariant makes the write window safe without
/// a lock.
static mut TASK_REGISTRY: [Option<TaskDescriptor>; MAX_TASKS] =
    [const { None }; MAX_TASKS];

/// Number of tasks currently registered.
static mut TASK_COUNT: usize = 0;

/// Register a task descriptor at boot time.
///
/// Appends `desc` to the next free slot in `TASK_REGISTRY`.
///
/// # Panics
///
/// Panics (via the kernel panic handler) if the registry is full
/// (`TASK_COUNT == MAX_TASKS`).
///
/// # Safety invariant
///
/// Must only be called from the boot path, before the scheduler starts and
/// before machine-mode interrupts are enabled.  Concurrent calls (re-entrant
/// or from an ISR) would produce a data race.
pub fn register_task(desc: TaskDescriptor) {
    // SAFETY: called exclusively from the single-threaded boot path, before
    // interrupts are enabled.  No other code aliases TASK_REGISTRY or
    // TASK_COUNT at this point.  Raw pointers via addr_of_mut! avoid creating
    // a Rust reference to mutable statics, which is UB-prone under the
    // Rust 2024 rules.
    unsafe {
        let count = *core::ptr::addr_of!(TASK_COUNT);
        assert!(
            count < MAX_TASKS,
            "register_task: registry full (MAX_TASKS reached)"
        );
        (*core::ptr::addr_of_mut!(TASK_REGISTRY))[count] = Some(desc);
        *core::ptr::addr_of_mut!(TASK_COUNT) = count + 1;
    }
}

/// Return an immutable slice of the registered tasks.
///
/// Valid to call after the boot registration phase.  Returns a slice of
/// length `MAX_TASKS`; unregistered slots are `None`.
///
/// # Safety invariant
///
/// Must not be called concurrently with [`register_task`].  Safe to call
/// once the boot phase is complete (interrupts enabled, scheduler running).
pub fn registry() -> &'static [Option<TaskDescriptor>; MAX_TASKS] {
    // SAFETY: after boot registration is complete, TASK_REGISTRY is only
    // ever read — never written — so the raw-pointer-to-reference cast is
    // sound.  addr_of! avoids creating a reference to the mutable static
    // directly (which is UB-prone per Rust 2024 static-mut-refs rules).
    unsafe { &*core::ptr::addr_of!(TASK_REGISTRY) }
}

/// Return the number of registered tasks.
pub fn task_count() -> usize {
    // SAFETY: same invariant as `registry()`.
    unsafe { *core::ptr::addr_of!(TASK_COUNT) }
}

/// Return a raw mutable pointer to the task registry for boot-time MIP writes.
///
/// # Safety invariant
///
/// The caller must ensure exclusive access — no other code may alias
/// `TASK_REGISTRY` while this pointer is in use.  Only call from the
/// single-threaded boot path before interrupts are enabled.
///
/// # Why a fn pointer?
///
/// Returning `*mut` directly from a safe function would allow callers to
/// create the pointer without acknowledging the safety obligation.
/// The caller marks its own `unsafe` block and documents the invariant there.
pub fn task_registry_ptr() -> *mut [Option<TaskDescriptor>; MAX_TASKS] {
    core::ptr::addr_of_mut!(TASK_REGISTRY)
}
