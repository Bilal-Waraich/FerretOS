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
const CLINT_BASE: usize = 0x0200_0000;
/// `mtime` low 32 bits — byte offset from CLINT base.
const MTIME_LO:   usize = 0x0000_BFF8;
/// `mtime` high 32 bits — byte offset from CLINT base.
const MTIME_HI:   usize = 0x0000_BFFC;
/// `mtimecmp` low 32 bits (hart 0) — byte offset from CLINT base.
const MTIMECMP_LO: usize = 0x0000_4000;
/// `mtimecmp` high 32 bits (hart 0) — byte offset from CLINT base.
const MTIMECMP_HI: usize = 0x0000_4004;

/// Timer ticks per 1 ms at the QEMU virt 10 MHz timebase.
pub const TICK_CYCLES: u64 = 10_000;

/// Monotonic tick counter incremented by the timer ISR.
///
/// `AtomicU32` gives lock-free read access from both the ISR and the main
/// loop without needing a critical section on a single-hart system.
static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Read the 64-bit `mtime` counter.
///
/// On RV32, `mtime` is a 64-bit register split across two 32-bit MMIO words.
/// The high word must be re-read after the low word to detect a carry between
/// reads — the standard two-read idiom from the privileged spec.
///
/// # Time complexity: O(1)
pub fn get_mtime() -> u64 {
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
    unsafe {
        ((CLINT_BASE + MTIMECMP_HI) as *mut u32).write_volatile(u32::MAX);
        ((CLINT_BASE + MTIMECMP_LO) as *mut u32).write_volatile(lo);
        ((CLINT_BASE + MTIMECMP_HI) as *mut u32).write_volatile(hi);
    }
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

/// Machine timer ISR — called from the assembly trap entry stub.
///
/// Increments the tick counter, reprints a tick line over UART, and
/// re-arms the timer.  The re-arm must happen before returning; failing to
/// do so causes the interrupt line to stay asserted and the CPU to re-enter
/// the handler immediately after `mret`.
pub fn timer_isr() {
    let tick = TICK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    crate::uart::uart_puts("[TICK ");
    crate::uart::uart_print_usize(tick as usize);
    crate::uart::uart_puts("]\n");
    schedule_tick(TICK_CYCLES);
}

/// Return the current tick count.
pub fn ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
