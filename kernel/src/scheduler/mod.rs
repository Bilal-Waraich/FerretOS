//! CA-PIP preemptive scheduler for FerretOS.
//!
//! # Architecture
//!
//! 1. **Build-time CCG + MIP** — `build.rs` parses `src/generated/demo_tasks.rs`,
//!    constructs the CCG, and runs BFS (with alloc, on the host) to produce
//!    `src/generated/ccg_constants.rs`.  Zero graph traversal happens at boot.
//! 2. **MIP write** (`write_precomputed_mip`) — O(N) copy from the const array
//!    into each `TaskDescriptor.max_inherited_priority`.
//! 3. **Ready queue** (`queue.rs`) — static max-heap keyed by `effective_priority`.
//! 4. **Dispatch** (this file) — the trap path saves the running task's frame
//!    pointer, then [`on_timer_interrupt`], [`yield_current`] or
//!    [`sleep_current`] decide which task's frame to resume.  Returning a
//!    different frame to the trap stub *is* the context switch.
//! 5. **Preemption rule** — on each tick, switch when the head of the ready
//!    queue has a strictly higher effective priority than the running task, or
//!    when the running task's quantum has expired and an equal-priority task is
//!    waiting (round-robin; the queue is FIFO among equal priorities).
//!
//! `ccg.rs` and `mip.rs` hold the reference (in-kernel) CCG and MIP
//! algorithms.  They are compiled only for host unit tests and Kani, where
//! they cross-check the build-time generator (`kernel/build/ccg_gen.rs`); the
//! kernel binary contains no graph code.
//!
//! # CA-PIP guarantee
//!
//! Because `max_inherited_priority` is precomputed and never changes at runtime,
//! priority inversion is structurally impossible: a holder's effective priority
//! is always ≥ every waiter's priority, so it can never be preempted by a waiter.

#[cfg(any(test, kani))]
pub mod ccg;
#[cfg(any(test, kani))]
pub mod mip;
pub mod queue;

#[cfg(test)]
mod tests;

use crate::config::MAX_TASKS;
use crate::context::TrapFrame;
use crate::generated::ccg_constants::MAX_INHERITED_PRIORITIES;
use crate::memory::task::{task_count, task_registry_ptr, TaskDescriptor, TaskMode, TaskState};
use queue::PriorityQueue;

/// Time quantum in timer ticks (1 tick = 1 ms by default).
///
/// A task that has held the CPU for this many ticks without yielding is moved
/// to the back of its priority level (round-robin tie-breaking).
pub const TIME_QUANTUM_TICKS: u32 = 5;

/// Maximum context switch latency in CLINT cycles before a warning is logged.
///
/// At 10 MHz that is 100 µs.  Exceeding this suggests the trap entry/exit path
/// has grown beyond the Sprint 4 budget.
pub const SWITCH_LATENCY_WARN_CYCLES: u32 = 1_000;

/// Number of context switches to accumulate before printing latency stats.
const SWITCH_STATS_WINDOW: u32 = 100;

// ---------------------------------------------------------------------------
// Context-switch latency statistics (Issue #41)
// ---------------------------------------------------------------------------

/// Running min/max/total statistics over a sliding window of context switches.
///
/// After `SWITCH_STATS_WINDOW` samples the stats are printed to UART and reset.
struct SwitchStats {
    min: u32,
    max: u32,
    total: u64,
    count: u32,
}

impl SwitchStats {
    const fn new() -> Self {
        SwitchStats { min: u32::MAX, max: 0, total: 0, count: 0 }
    }

    /// Record one context-switch latency sample in CLINT cycles.
    fn record(&mut self, cycles: u32) {
        if cycles < self.min { self.min = cycles; }
        if cycles > self.max { self.max = cycles; }
        self.total += cycles as u64;
        self.count += 1;

        if self.count >= SWITCH_STATS_WINDOW {
            let avg = (self.total / self.count as u64) as u32;
            #[cfg(not(any(test, kani)))]
            {
                use crate::uart;
                uart::uart_puts("Switch latency: min=");
                uart::uart_print_usize(self.min as usize);
                uart::uart_puts(" max=");
                uart::uart_print_usize(self.max as usize);
                uart::uart_puts(" avg=");
                uart::uart_print_usize(avg as usize);
                uart::uart_puts(" cycles");
                if self.max > SWITCH_LATENCY_WARN_CYCLES {
                    uart::uart_puts(" [WARN: exceeded budget]");
                }
                uart::uart_puts("\n");
            }
            let _ = avg; // suppress unused warning in test builds
            // Reset for next window.
            *self = SwitchStats::new();
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler state — populated at boot, used by the trap path.
// ---------------------------------------------------------------------------

/// Registry index meaning "no task is running" (boot, before the first dispatch).
pub const NO_TASK: u8 = 0xFF;

/// Global ready queue — max-heap keyed by effective priority, FIFO among equals.
static mut READY_QUEUE: PriorityQueue<MAX_TASKS> = PriorityQueue::new();

/// Registry index of the currently running task.
static mut CURRENT_TASK_IDX: u8 = NO_TASK;

/// Number of ticks the current task has run without being switched out.
static mut TICKS_SINCE_SWITCH: u32 = 0;

/// Context-switch latency statistics accumulator.
static mut SWITCH_STATS: SwitchStats = SwitchStats::new();

/// Raw pointer to the descriptor in registry slot `idx`, or null if empty.
///
/// The runtime scheduler mutates descriptors (state, saved frame, wake tick)
/// through raw pointers only, never through the `&'static` slice returned by
/// [`crate::memory::task::registry`], so no shared reference is alive across a write.
fn task_ptr(idx: u8) -> *mut TaskDescriptor {
    if idx as usize >= MAX_TASKS {
        return core::ptr::null_mut();
    }
    // SAFETY: `idx < MAX_TASKS`, so the element pointer is in bounds.  Only a
    // raw pointer is formed; callers dereference it on the single hart with
    // interrupts disabled.
    unsafe {
        let slot = core::ptr::addr_of_mut!((*task_registry_ptr())[idx as usize]);
        match (*slot).as_mut() {
            Some(t) => t as *mut TaskDescriptor,
            None => core::ptr::null_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// Boot-time initialisation
// ---------------------------------------------------------------------------

/// Write precomputed MIP values from `ccg_constants` into the task registry.
///
/// build.rs parsed demo_tasks.rs, built the CCG, and ran BFS on the host
/// (with alloc) to populate `MAX_INHERITED_PRIORITIES`.  This call simply
/// copies those values — O(N) writes, zero graph traversal at boot.
fn write_precomputed_mip() {
    for (i, &mip) in MAX_INHERITED_PRIORITIES.iter().enumerate().take(task_count()) {
        let t = task_ptr(i as u8);
        if !t.is_null() {
            // SAFETY: single-threaded boot path; `t` points into the registry.
            unsafe { (*t).max_inherited_priority = mip };
        }
    }
}

/// Initialise the scheduler subsystem.
///
/// 1. Writes precomputed MIP into every `TaskDescriptor` (from build.rs constants).
/// 2. Enqueues all `Ready` tasks into the ready queue.
/// 3. Selects the highest-priority task as the first to run.
///
/// # Safety invariant
///
/// Must be called after all tasks are registered and before interrupts are
/// enabled (single-threaded boot path).
pub fn init() {
    write_precomputed_mip();

    // SAFETY: single-threaded boot path; no trap can fire yet.
    unsafe {
        let rq = core::ptr::addr_of_mut!(READY_QUEUE);
        for i in 0..task_count() {
            let t = task_ptr(i as u8);
            if !t.is_null() && (*t).state == TaskState::Ready {
                (*rq).insert(i as u8, (*t).effective_priority());
            }
        }
        if let Some(idx) = (*rq).pop_max() {
            mark_running(idx);
        }
    }
}

/// Build every registered task's initial trap frame on its kernel stack.
///
/// `exit_trap` is installed as each task's return address and `gp` is the
/// kernel's global pointer (tasks are linked into the kernel image).  Each
/// task starts with interrupts enabled (`MPIE`) in its declared
/// [`TaskMode`]: `MPP = U` for user tasks, `MPP = M` for kernel tasks.
///
/// # Safety
///
/// Boot path only, before the first task is started.  Every registered task
/// must have a non-zero `entry` and its own stack at
/// `[stack_base, stack_base + stack_size)` that nothing else uses.
pub unsafe fn prepare_frames(exit_trap: usize, gp: usize) {
    use crate::context::{kernel_stack_top, MSTATUS_MPIE, MSTATUS_MPP_MACHINE, MSTATUS_MPP_USER};
    for i in 0..task_count() {
        let t = task_ptr(i as u8);
        if t.is_null() {
            continue;
        }
        // SAFETY: boot path; `t` points into the registry, the kernel stack
        // for slot `i` is unused until this task first runs, and the caller
        // guarantees the task's own stack bounds and entry point.
        unsafe {
            let mpp = match (*t).mode {
                TaskMode::User => MSTATUS_MPP_USER,
                TaskMode::Machine => MSTATUS_MPP_MACHINE,
            };
            let frame = crate::context::init_frame(
                kernel_stack_top(i),
                (*t).stack_base + (*t).stack_size,
                (*t).entry,
                exit_trap,
                gp,
                mpp | MSTATUS_MPIE,
            );
            (*t).stack_ptr = frame as usize;
        }
    }
}

/// Bitmask of tasks whose first syscall has been reported.
static mut SYSCALL_SEEN: u32 = 0;

/// Report, once per task, the privilege mode its syscalls arrive from.
///
/// `from_user` is true for `ecall` from U-mode (`mcause` 8).  This is the
/// observable evidence — checked by the boot integration test — that tasks
/// really execute in the mode they were declared with.
pub fn note_syscall_mode(from_user: bool) {
    let idx = current_task_idx();
    if idx as usize >= 32 {
        return;
    }
    // SAFETY: trap path, interrupts disabled, single hart.
    unsafe {
        let seen = core::ptr::addr_of_mut!(SYSCALL_SEEN);
        if *seen & (1 << idx) != 0 {
            return;
        }
        *seen |= 1 << idx;
    }
    crate::uart::uart_puts("[KERNEL] task ");
    crate::uart::uart_print_usize(idx as usize);
    crate::uart::uart_puts(if from_user { " running in U-mode\n" } else { " running in M-mode\n" });
}

/// Saved frame of the task selected by [`init`], for the first dispatch.
pub fn first_frame() -> *mut TrapFrame {
    frame_of(current_task_idx())
}

// ---------------------------------------------------------------------------
// Runtime — called from the trap path with interrupts disabled.
// ---------------------------------------------------------------------------

/// Saved frame pointer of task `idx`.
fn frame_of(idx: u8) -> *mut TrapFrame {
    let t = task_ptr(idx);
    if t.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `t` points into the registry; read on the trap path.
    unsafe { (*t).stack_ptr as *mut TrapFrame }
}

/// Record `frame` as the running task's saved context.
fn save_current(frame: *mut TrapFrame) {
    let t = task_ptr(current_task_idx());
    if !t.is_null() {
        // SAFETY: trap path, interrupts disabled, single hart.
        unsafe { (*t).stack_ptr = frame as usize };
    }
}

/// Make `idx` the running task.
fn mark_running(idx: u8) {
    // SAFETY: trap path or boot path; single hart, interrupts disabled.
    unsafe {
        *core::ptr::addr_of_mut!(CURRENT_TASK_IDX) = idx;
        *core::ptr::addr_of_mut!(TICKS_SINCE_SWITCH) = 0;
        let t = task_ptr(idx);
        if !t.is_null() {
            (*t).state = TaskState::Running;
        }
    }
}

/// Pop the best ready task and make it current; return its frame.
///
/// The kernel always registers an idle task, so the queue is never empty
/// once the current task has blocked.  An empty queue therefore means the
/// idle task is missing — a boot configuration error.
fn dispatch_next() -> *mut TrapFrame {
    // SAFETY: trap path, interrupts disabled, single hart.
    let next = unsafe { (*core::ptr::addr_of_mut!(READY_QUEUE)).pop_max() };
    match next {
        Some(idx) => {
            mark_running(idx);
            frame_of(idx)
        }
        None => {
            crate::uart::uart_puts("[FAULT] no runnable task (idle task missing)\nSystem halted.\n");
            crate::halt()
        }
    }
}

/// Return the running task to the ready queue.
fn requeue_current() {
    let idx = current_task_idx();
    let t = task_ptr(idx);
    if !t.is_null() {
        // SAFETY: trap path, interrupts disabled, single hart.
        unsafe {
            (*t).state = TaskState::Ready;
            (*core::ptr::addr_of_mut!(READY_QUEUE)).insert(idx, (*t).effective_priority());
        }
    }
}

/// Whether tick `now` has reached `deadline`, tolerant of `u32` wraparound.
fn tick_reached(now: u32, deadline: u32) -> bool {
    (now.wrapping_sub(deadline) as i32) >= 0
}

/// Move every sleeping task whose wake tick has arrived back to `Ready`.
///
/// O(N) over the registry, N ≤ MAX_TASKS.
fn wake_sleepers(now: u32) {
    for i in 0..task_count() {
        let t = task_ptr(i as u8);
        // SAFETY: trap path, interrupts disabled, single hart.
        unsafe {
            if !t.is_null() && (*t).state == TaskState::Blocked && tick_reached(now, (*t).wake_at) {
                (*t).state = TaskState::Ready;
                (*core::ptr::addr_of_mut!(READY_QUEUE)).insert(i as u8, (*t).effective_priority());
            }
        }
    }
}

/// Halt with a diagnostic if any task's stack canary has been overwritten.
///
/// Runs on every tick so an overflow is caught within one quantum (#69).
fn check_stack_canaries() {
    for i in 0..task_count() {
        let t = task_ptr(i as u8);
        if t.is_null() {
            continue;
        }
        // SAFETY: `stack_base` is the start of the task's `Stack<N>`, whose
        // first 4 bytes hold the canary (written by `Stack::new`).
        let intact = unsafe { crate::memory::stack::canary_intact((*t).stack_base) };
        if !intact {
            crate::uart::uart_puts("[FAULT] stack overflow: canary overwritten in task ");
            crate::uart::uart_print_usize(i);
            crate::uart::uart_puts("\nSystem halted.\n");
            crate::halt();
        }
    }
}

/// Decide whether the running task should be switched out on this tick.
///
/// Pure function of the scheduling state so it can be unit-tested:
/// switch when a strictly higher effective priority is waiting, or when the
/// quantum has expired and an equal-or-higher priority task is waiting.
pub fn should_preempt(cur_eff: u8, next_eff: Option<u8>, ticks_since_switch: u32) -> bool {
    match next_eff {
        None => false,
        Some(next) => {
            next > cur_eff || (ticks_since_switch >= TIME_QUANTUM_TICKS && next >= cur_eff)
        }
    }
}

/// Machine-timer interrupt: advance time, wake sleepers, maybe preempt.
///
/// Returns the frame to resume — `frame` itself, or the next task's frame.
pub fn on_timer_interrupt(frame: *mut TrapFrame) -> *mut TrapFrame {
    let isr_entry = crate::clint::get_mtime() as u32;
    let now = crate::clint::timer_isr();
    save_current(frame);
    check_stack_canaries();
    wake_sleepers(now);

    match tick(isr_entry) {
        Some(_) => frame_of(current_task_idx()),
        None => frame,
    }
}

/// Preemption check for one timer tick.
///
/// Returns `Some(new_task_idx)` when the scheduler switched away from the
/// current task, `None` when the current task continues.  Records the
/// decision latency (ISR entry → switch decided) for #41 statistics.
///
/// # Arguments
///
/// * `isr_entry` — low 32 bits of `mtime` sampled at ISR entry.
pub fn tick(isr_entry: u32) -> Option<u8> {
    // SAFETY: trap path, interrupts disabled, single hart; scheduler statics
    // are accessed only through raw pointers.
    unsafe {
        let ticks_ptr = core::ptr::addr_of_mut!(TICKS_SINCE_SWITCH);
        let rq = core::ptr::addr_of_mut!(READY_QUEUE);
        *ticks_ptr = (*ticks_ptr).wrapping_add(1);

        let cur_idx = current_task_idx();
        let cur = task_ptr(cur_idx);
        let cur_eff = if cur.is_null() { 0 } else { (*cur).effective_priority() };
        let next_eff = if (*rq).is_empty() { None } else { Some((*rq).peek_max_priority()) };

        if !should_preempt(cur_eff, next_eff, *ticks_ptr) {
            return None;
        }

        requeue_current();
        dispatch_next();
        let next_idx = current_task_idx();
        if next_idx == cur_idx {
            return None;
        }

        let elapsed = (crate::clint::get_mtime() as u32).wrapping_sub(isr_entry);
        (*core::ptr::addr_of_mut!(SWITCH_STATS)).record(elapsed);
        Some(next_idx)
    }
}

/// `SYS_YIELD`: requeue the caller behind its equal-priority peers and
/// dispatch the best ready task (possibly the caller again).
pub fn yield_current(frame: *mut TrapFrame) -> *mut TrapFrame {
    save_current(frame);
    requeue_current();
    dispatch_next()
}

/// `SYS_SLEEP`: block the caller for `ticks` ticks (minimum 1) and dispatch
/// the best ready task.
pub fn sleep_current(frame: *mut TrapFrame, ticks: u32) -> *mut TrapFrame {
    save_current(frame);
    let t = task_ptr(current_task_idx());
    if !t.is_null() {
        // SAFETY: trap path, interrupts disabled, single hart.
        unsafe {
            (*t).state = TaskState::Blocked;
            (*t).wake_at = crate::clint::ticks().wrapping_add(ticks.max(1));
        }
    }
    dispatch_next()
}

/// Return the registry index of the currently running task.
pub fn current_task_idx() -> u8 {
    // SAFETY: read of a single byte via raw pointer; no torn read possible on RV32.
    unsafe { *core::ptr::addr_of!(CURRENT_TASK_IDX) }
}
