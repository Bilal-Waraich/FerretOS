//! RISC-V CLINT (Core Local Interruptor) driver.
//!
//! The CLINT provides the `mtime` free-running counter and the `mtimecmp`
//! compare register.  When `mtime >= mtimecmp` the CLINT asserts the machine
//! timer interrupt (MTIP).
//!
//! MMIO map for QEMU `virt` (riscv32):
//!
//! | Register  | Address        | Width  | Access |
//! |-----------|----------------|--------|--------|
//! | `msip`    | `0x0200_0000`  | 32-bit | R/W    |
//! | `mtimecmp`| `0x0200_4000`  | 64-bit | R/W    |
//! | `mtime`   | `0x0200_BFF8`  | 64-bit | R      |

use core::sync::atomic::{AtomicU32, Ordering};

/// CLINT base address on QEMU `virt`.
#[cfg(target_arch = "riscv32")]
const CLINT_BASE: usize = 0x0200_0000;
/// `mtime` low 32 bits — byte offset from CLINT base.
#[cfg(target_arch = "riscv32")]
const MTIME_LO:   usize = 0x0000_BFF8;
/// `mtime` high 32 bits — byte offset from CLINT base.
#[cfg(target_arch = "riscv32")]
const MTIME_HI:   usize = 0x0000_BFFC;
/// `mtimecmp` low 32 bits (hart 0) — byte offset from CLINT base.
#[cfg(target_arch = "riscv32")]
const MTIMECMP_LO: usize = 0x0000_4000;
/// `mtimecmp` high 32 bits (hart 0) — byte offset from CLINT base.
#[cfg(target_arch = "riscv32")]
const MTIMECMP_HI: usize = 0x0000_4004;

/// Timer ticks per 1 ms at the QEMU virt 10 MHz timebase.
pub const TICK_CYCLES: u64 = 10_000;

/// Monotonic tick counter incremented by the timer ISR.
///
/// `AtomicU32` gives lock-free read access from both the ISR and the main
/// loop without needing a critical section on a single-hart system.
static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Absolute `mtime` value of the next scheduled tick.
static mut NEXT_DEADLINE: u64 = 0;

/// Read the 64-bit `mtime` counter.
///
/// On RV32, `mtime` is a 64-bit register split across two 32-bit MMIO words.
/// The high word must be re-read after the low word to detect a carry between
/// reads — the standard two-read idiom from the privileged spec.
///
/// # Time complexity: O(1)
pub fn get_mtime() -> u64 {
    #[cfg(target_arch = "riscv32")]
    loop {
        // SAFETY: CLINT_BASE + MTIME_HI/LO are valid MMIO addresses for QEMU
        // virt.  Volatile reads are required because the hardware updates these
        // registers asynchronously with respect to the CPU pipeline.
        let hi0 = unsafe {
            ((CLINT_BASE + MTIME_HI) as *const u32).read_volatile()
        };
        let lo = unsafe {
            ((CLINT_BASE + MTIME_LO) as *const u32).read_volatile()
        };
        let hi1 = unsafe {
            ((CLINT_BASE + MTIME_HI) as *const u32).read_volatile()
        };
        // Re-read if a carry propagated from lo to hi between the two reads.
        if hi0 == hi1 {
            return ((hi0 as u64) << 32) | (lo as u64);
        }
    }
    // Host builds (unit tests, Kani) have no CLINT; time stands still.
    #[cfg(not(target_arch = "riscv32"))]
    0
}

/// Write a new deadline to `mtimecmp` for hart 0.
///
/// Per the RISC-V privileged spec: to avoid a spurious interrupt when
/// updating a 64-bit compare value on a 32-bit bus, write `u32::MAX` to the
/// high word first (raising the effective deadline above any current `mtime`),
/// then write the low word, then the final high word.
///
/// # Time complexity: O(1)
pub fn set_mtimecmp(val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    // SAFETY: CLINT_BASE + MTIMECMP_{HI,LO} are valid MMIO addresses.
    // Volatile writes are required to prevent reordering or elision by the
    // compiler; the order (hi_max → lo → hi) is architecturally mandated.
    #[cfg(target_arch = "riscv32")]
    unsafe {
        ((CLINT_BASE + MTIMECMP_HI) as *mut u32).write_volatile(u32::MAX);
        ((CLINT_BASE + MTIMECMP_LO) as *mut u32).write_volatile(lo);
        ((CLINT_BASE + MTIMECMP_HI) as *mut u32).write_volatile(hi);
    }
    #[cfg(not(target_arch = "riscv32"))]
    let _ = (lo, hi);
}

/// Schedule the next timer tick `cycles` ticks from now.
///
/// Sets `mtimecmp = mtime + cycles`.
///
/// # Time complexity: O(1) amortised (the `get_mtime` carry-loop is
///   effectively always 1 iteration)
pub fn schedule_tick(cycles: u64) {
    set_mtimecmp(get_mtime() + cycles);
}

/// Advance the tick counter and re-arm the timer for the next tick.
///
/// Called from the machine-timer trap path before the scheduler runs.  The
/// re-arm must happen before returning; failing to do so leaves MTIP asserted
/// and the hart re-enters the trap immediately after `mret`.
///
/// The next deadline is the previous deadline plus one tick, not `now + 1
/// tick`, so time spent in the trap handler does not accumulate as drift.
pub fn timer_isr() -> u32 {
    let tick = TICK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    // SAFETY: NEXT_DEADLINE is only touched here and in `start_ticks`, both
    // on the single hart with interrupts disabled.
    unsafe {
        let next = core::ptr::addr_of_mut!(NEXT_DEADLINE);
        *next += TICK_CYCLES;
        // If the handler fell more than a tick behind, resynchronise rather
        // than firing a burst of back-to-back catch-up interrupts.
        let now = get_mtime();
        if *next <= now {
            *next = now + TICK_CYCLES;
        }
        set_mtimecmp(*next);
    }
    tick
}

/// Arm the first timer tick one `TICK_CYCLES` period from now.
///
/// Must be called once on the boot path before interrupts are enabled.
pub fn start_ticks() {
    // SAFETY: boot path, interrupts disabled, single hart.
    unsafe {
        let next = core::ptr::addr_of_mut!(NEXT_DEADLINE);
        *next = get_mtime() + TICK_CYCLES;
        set_mtimecmp(*next);
    }
}

/// Read the low 32 bits of `minstret`, the retired-instruction counter.
///
/// Used to measure kernel path lengths.  Unlike `mtime`, which QEMU advances
/// with host wall-clock time, `minstret` counts the guest's own instructions,
/// so measurements are deterministic and independent of host load.  Returns
/// 0 on non-RISC-V hosts.
pub fn instret() -> u32 {
    #[cfg(target_arch = "riscv32")]
    {
        let v: u32;
        // SAFETY: reading minstret has no side effects; M-mode only.
        unsafe { core::arch::asm!("csrr {}, minstret", out(reg) v, options(nomem, nostack)) };
        v
    }
    #[cfg(not(target_arch = "riscv32"))]
    0
}

/// Return the current tick count.
pub fn ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
