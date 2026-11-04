//! CA-PIP three-task demo (H / M / L) plus the kernel idle task.
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
//! Every log line carries the tick at which it was written so
//! `tests/integration/boot_test.sh` can check the guarantee mechanically:
//! no `[M] run` or `[H] run` tick may fall strictly inside an L work window.

use ferret::sys::{self, LineBuf};

/// Busy-wait (without yielding) until tick `t`.
fn work_until(t: u32) {
    while sys::now() < t {}
}

/// Task L — low base priority, holds UART0, inherits priority 3.
pub fn task_l() -> ! {
    loop {
        let start = sys::now();
        LineBuf::<64>::new().str("[L] work start t=").u32(start).str(" (holds UART0, eff_pri=3)\n").log();
        work_until(start + 3);
        LineBuf::<64>::new().str("[L] work end t=").u32(sys::now()).str("\n").log();
        sys::sleep(20);
    }
}

/// Task M — medium priority, CPU-bound, no capabilities.
pub fn task_m() -> ! {
    loop {
        let start = sys::now();
        LineBuf::<64>::new().str("[M] run t=").u32(start).str("\n").log();
        work_until(start + 2);
        sys::sleep(7);
    }
}

/// Task H — high priority, requires UART0.
pub fn task_h() -> ! {
    loop {
        LineBuf::<64>::new().str("[H] run t=").u32(sys::now()).str("\n").log();
        sys::sleep(10);
    }
}

/// Idle task — runs when nothing else is ready; waits for the next interrupt.
pub fn idle() -> ! {
    loop {
        // SAFETY: `wfi` only pauses the hart until an interrupt is pending.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Return address installed in every task's initial frame.
///
/// Task bodies are `fn() -> !`, so reaching this is a bug; report it and park
/// the task forever without stopping the rest of the system.
pub fn task_returned() -> ! {
    sys::log(b"[KERNEL] task returned from its entry point; parking it\n");
    loop {
        sys::sleep(u32::MAX / 2);
    }
}
