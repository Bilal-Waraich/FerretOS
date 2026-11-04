//! CA-PIP three-task demo (H / M / L), selected with `--features demo-hml`.
//!
//! Peripheral bitmasks: UART0 = bit 0.
//!
//! | Task | Base | Caps | MIP | Effective |
//! |---|---|---|---|---|
//! | L | 1 | holds UART0 | 3 | 3 |
//! | M | 2 | none | 2 | 2 |
//! | H | 3 | requires UART0 | 3 | 3 |
//!
//! L performs a 3-tick unit of work while holding UART0.  Because L's
//! effective priority (3) exceeds M's (2), M can never preempt L during that
//! window, and because H's priority is equal to L's and the work is shorter
//! than the quantum, H also waits until L finishes — the CA-PIP guarantee
//! that a holder is never delayed by a lower-priority task.
//!
//! All three tasks hold the shared console capability (bit 31), which the
//! `log` syscall requires.  On its first run M also runs the syscall
//! validation probe (see `super::probe_log_validation`).
//!
//! Every log line carries the tick at which it was written so
//! `tests/integration/boot_test.sh` can check the guarantee mechanically:
//! no `[M] run` or `[H] run` tick may fall strictly inside an L work window.

use super::claim_stack;
use ferret::generated::ccg_constants::HML_MAX_INHERITED_PRIORITIES;
use ferret::generated::demo_tasks::{TASK_H, TASK_L, TASK_M};
use ferret::memory::{self, register_task};
use ferret::config::MAX_TASKS;
use ferret::sys::{self, LineBuf};

/// Busy-wait (without yielding) until tick `t`.
fn work_until(t: u32) {
    while sys::now() < t {}
}

/// Task L — low base priority, holds UART0, inherits priority 3.
pub fn task_l() -> ! {
    loop {
        let start = sys::now();
        let _ = LineBuf::<64>::new().str("[L] work start t=").u32(start).str(" (holds UART0, eff_pri=3)\n").log();
        work_until(start + 3);
        let _ = LineBuf::<64>::new().str("[L] work end t=").u32(sys::now()).str("\n").log();
        sys::sleep(20);
    }
}

/// Task M — medium priority, CPU-bound, no peripheral capabilities.
pub fn task_m() -> ! {
    super::probe_log_validation();
    loop {
        let start = sys::now();
        let _ = LineBuf::<64>::new().str("[M] run t=").u32(start).str("\n").log();
        work_until(start + 2);
        sys::sleep(7);
    }
}

/// Task H — high priority, requires UART0.
pub fn task_h() -> ! {
    loop {
        let _ = LineBuf::<64>::new().str("[H] run t=").u32(sys::now()).str("\n").log();
        sys::sleep(10);
    }
}

// Each stack fills its task's 4 KB region in TASKRAM (slot order).
#[link_section = ".task_ram.0"]
static mut STACK_TASK_L: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.1"]
static mut STACK_TASK_M: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.2"]
static mut STACK_TASK_H: memory::Stack<4096> = memory::Stack::new();

/// Register L, M and H in slot order and return their precomputed MIP table.
pub fn register() -> &'static [u8; MAX_TASKS] {
    register_task(TASK_L.into_descriptor(0, claim_stack(core::ptr::addr_of!(STACK_TASK_L) as usize), task_l));
    register_task(TASK_M.into_descriptor(1, claim_stack(core::ptr::addr_of!(STACK_TASK_M) as usize), task_m));
    register_task(TASK_H.into_descriptor(2, claim_stack(core::ptr::addr_of!(STACK_TASK_H) as usize), task_h));
    &HML_MAX_INHERITED_PRIORITIES
}
