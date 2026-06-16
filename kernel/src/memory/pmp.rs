//! RISC-V PMP (Physical Memory Protection) register configuration.
//!
//! At boot, after the task registry is finalised, `configure_pmp()` writes one
//! PMP entry per task memory region using TOR (Top Of Range) mode.  Each entry
//! grants read+write access to the task's `[memory_start, memory_end)` range
//! in Machine mode; User-mode access is not enabled (kernel-only target).
//!
//! # PMP entry layout (RV32)
//!
//! Each entry occupies one `pmpaddr` CSR and 8 bits of a `pmpcfg` CSR:
//!
//! ```
//! pmpcfgN bits: [L | 00 | A[1:0] | X | W | R]
//!   R  = readable
//!   W  = writable
//!   X  = executable
//!   A  = address mode: 00=OFF, 01=TOR, 10=NA4, 11=NAPOT
//!   L  = locked (write-once; we leave it unlocked for now)
//! ```
//!
//! TOR mode: entry i matches addresses `pmpaddr[i-1] <= addr < pmpaddr[i]`.
//! We use pairs: even entry = base (addr >> 2), odd entry = limit (addr >> 2),
//! with the odd entry carrying the TOR config byte.
//!
//! # Feature gate
//!
//! This module is compiled only when the `pmp` Cargo feature is enabled:
//!
//! ```
//! cargo build --release --features pmp
//! ```
//!
//! The baseline kernel operates without PMP; enabling it adds hardware-enforced
//! isolation at the cost of boot time proportional to the number of tasks.
//!
//! # Limitations
//!
//! - RV32 supports up to 16 PMP entries (pmpaddr0–pmpaddr15, pmpcfg0–pmpcfg3).
//! - We use 2 entries per task, so at most 8 tasks can have PMP entries.
//!   Tasks beyond that are left unprotected (a warning is printed to UART).
//! - Addresses must be 4-byte aligned (the bottom 2 bits are always 0 on RV32).

#[cfg(feature = "pmp")]
pub use inner::configure_pmp;

#[cfg(not(feature = "pmp"))]
pub fn configure_pmp() {
    // No-op when the pmp feature is not enabled.
}

#[cfg(feature = "pmp")]
mod inner {
    use crate::memory::task::registry;
    #[cfg(not(test))]
    use crate::uart;

    /// Maximum number of PMP entries available on RV32 RISC-V.
    const PMP_ENTRIES: usize = 16;
    /// Entries per task: one for the base address, one TOR entry for the limit.
    const ENTRIES_PER_TASK: usize = 2;
    /// Maximum tasks we can protect with PMP entries.
    pub const MAX_PMP_TASKS: usize = PMP_ENTRIES / ENTRIES_PER_TASK; // 8

    // pmpcfg byte values.
    const PMP_R:   u8 = 1 << 0; // readable
    const PMP_W:   u8 = 1 << 1; // writable
    const PMP_TOR: u8 = 1 << 3; // address mode: Top Of Range
    const PMP_OFF: u8 = 0;      // disabled entry

    /// Configure PMP registers to match the task memory regions.
    ///
    /// Must be called after `register_task()` finishes (task registry is final)
    /// and before the first task runs.  Leaves PMP entries beyond the registered
    /// task count as OFF.
    ///
    /// # Safety invariant
    ///
    /// Must run on the single-threaded boot path before interrupts are enabled.
    /// CSR writes are inherently machine-mode-only; no other hart may be active.
    pub fn configure_pmp() {
        let reg = registry();

        // Collect (start, end) pairs from registered tasks.
        let mut regions: [(usize, usize); MAX_PMP_TASKS] = [(0, 0); MAX_PMP_TASKS];
        let mut count = 0;

        for task in reg.iter().flatten() {
            if count >= MAX_PMP_TASKS {
                #[cfg(not(test))]
                {
                    uart::uart_puts(
                        "WARNING: PMP: more tasks than PMP entries — \
                         tasks beyond slot 7 are unprotected.\n",
                    );
                }
                break;
            }
            regions[count] = (task.memory_start, task.memory_end);
            count += 1;
        }

        // Build the pmpaddr and pmpcfg arrays for all 16 entries.
        // Entry 2i   = base address (OFF mode, acts as lower bound for TOR).
        // Entry 2i+1 = limit address (TOR mode, R+W permissions).
        let mut pmpaddr = [0usize; PMP_ENTRIES];
        let mut pmpcfg  = [PMP_OFF; PMP_ENTRIES];

        for i in 0..count {
            let (start, end) = regions[i];
            // PMP addresses are physical address >> 2 on RV32.
            pmpaddr[2 * i]     = start >> 2;
            pmpaddr[2 * i + 1] = end   >> 2;
            pmpcfg[2 * i]      = PMP_OFF;              // base sentinel, no perms
            pmpcfg[2 * i + 1]  = PMP_TOR | PMP_R | PMP_W; // TOR R+W
        }

        // Write pmpaddr CSRs.  RV32 has pmpaddr0–pmpaddr15.
        // SAFETY: CSR writes are M-mode privileged operations; we are in
        // machine mode on the single-threaded boot path.
        unsafe {
            write_pmpaddr(0,  pmpaddr[0]);
            write_pmpaddr(1,  pmpaddr[1]);
            write_pmpaddr(2,  pmpaddr[2]);
            write_pmpaddr(3,  pmpaddr[3]);
            write_pmpaddr(4,  pmpaddr[4]);
            write_pmpaddr(5,  pmpaddr[5]);
            write_pmpaddr(6,  pmpaddr[6]);
            write_pmpaddr(7,  pmpaddr[7]);
            write_pmpaddr(8,  pmpaddr[8]);
            write_pmpaddr(9,  pmpaddr[9]);
            write_pmpaddr(10, pmpaddr[10]);
            write_pmpaddr(11, pmpaddr[11]);
            write_pmpaddr(12, pmpaddr[12]);
            write_pmpaddr(13, pmpaddr[13]);
            write_pmpaddr(14, pmpaddr[14]);
            write_pmpaddr(15, pmpaddr[15]);
        }

        // Pack 4 pmpcfg bytes into each 32-bit pmpcfg CSR.
        // pmpcfg0 = entries 0–3, pmpcfg1 = entries 4–7, etc.
        let pack = |i: usize| -> usize {
            (pmpcfg[4 * i] as usize)
                | ((pmpcfg[4 * i + 1] as usize) << 8)
                | ((pmpcfg[4 * i + 2] as usize) << 16)
                | ((pmpcfg[4 * i + 3] as usize) << 24)
        };

        // SAFETY: same as above.
        unsafe {
            write_pmpcfg(0, pack(0));
            write_pmpcfg(1, pack(1));
            write_pmpcfg(2, pack(2));
            write_pmpcfg(3, pack(3));
        }

        #[cfg(not(test))]
        {
            uart::uart_puts("PMP configured: ");
            uart::uart_print_usize(count);
            uart::uart_puts(" region(s) protected.\n");
        }
    }

    // ---------------------------------------------------------------------------
    // CSR write helpers — one inline asm per register (required on RV32 where
    // the CSR address must be an immediate in the csrw instruction).
    // ---------------------------------------------------------------------------

    macro_rules! write_pmpaddr_impl {
        ($n:literal, $csr:literal) => {
            // SAFETY: caller guarantees M-mode boot path.
            unsafe fn $n(val: usize) {
                core::arch::asm!(
                    concat!("csrw ", $csr, ", {v}"),
                    v = in(reg) val,
                    options(nomem, nostack),
                );
            }
        };
    }

    write_pmpaddr_impl!(pmpaddr_0,  "pmpaddr0");
    write_pmpaddr_impl!(pmpaddr_1,  "pmpaddr1");
    write_pmpaddr_impl!(pmpaddr_2,  "pmpaddr2");
    write_pmpaddr_impl!(pmpaddr_3,  "pmpaddr3");
    write_pmpaddr_impl!(pmpaddr_4,  "pmpaddr4");
    write_pmpaddr_impl!(pmpaddr_5,  "pmpaddr5");
    write_pmpaddr_impl!(pmpaddr_6,  "pmpaddr6");
    write_pmpaddr_impl!(pmpaddr_7,  "pmpaddr7");
    write_pmpaddr_impl!(pmpaddr_8,  "pmpaddr8");
    write_pmpaddr_impl!(pmpaddr_9,  "pmpaddr9");
    write_pmpaddr_impl!(pmpaddr_10, "pmpaddr10");
    write_pmpaddr_impl!(pmpaddr_11, "pmpaddr11");
    write_pmpaddr_impl!(pmpaddr_12, "pmpaddr12");
    write_pmpaddr_impl!(pmpaddr_13, "pmpaddr13");
    write_pmpaddr_impl!(pmpaddr_14, "pmpaddr14");
    write_pmpaddr_impl!(pmpaddr_15, "pmpaddr15");

    macro_rules! write_pmpcfg_impl {
        ($n:literal, $csr:literal) => {
            unsafe fn $n(val: usize) {
                core::arch::asm!(
                    concat!("csrw ", $csr, ", {v}"),
                    v = in(reg) val,
                    options(nomem, nostack),
                );
            }
        };
    }

    write_pmpcfg_impl!(pmpcfg_0, "pmpcfg0");
    write_pmpcfg_impl!(pmpcfg_1, "pmpcfg1");
    write_pmpcfg_impl!(pmpcfg_2, "pmpcfg2");
    write_pmpcfg_impl!(pmpcfg_3, "pmpcfg3");

    /// Dispatch pmpaddr write to the correct CSR by entry index.
    ///
    /// SAFETY: caller must ensure M-mode boot-path invariant.
    unsafe fn write_pmpaddr(idx: usize, val: usize) {
        // SAFETY: delegated to the per-index functions above.
        unsafe {
            match idx {
                0  => pmpaddr_0(val),
                1  => pmpaddr_1(val),
                2  => pmpaddr_2(val),
                3  => pmpaddr_3(val),
                4  => pmpaddr_4(val),
                5  => pmpaddr_5(val),
                6  => pmpaddr_6(val),
                7  => pmpaddr_7(val),
                8  => pmpaddr_8(val),
                9  => pmpaddr_9(val),
                10 => pmpaddr_10(val),
                11 => pmpaddr_11(val),
                12 => pmpaddr_12(val),
                13 => pmpaddr_13(val),
                14 => pmpaddr_14(val),
                15 => pmpaddr_15(val),
                _  => {}
            }
        }
    }

    /// Dispatch pmpcfg write to the correct CSR by register index (0–3).
    ///
    /// SAFETY: caller must ensure M-mode boot-path invariant.
    unsafe fn write_pmpcfg(idx: usize, val: usize) {
        // SAFETY: delegated to the per-index functions above.
        unsafe {
            match idx {
                0 => pmpcfg_0(val),
                1 => pmpcfg_1(val),
                2 => pmpcfg_2(val),
                3 => pmpcfg_3(val),
                _ => {}
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn pmp_address_encoding() {
            // PMP address = physical address >> 2 on RV32.
            let addr = 0x8008_1000usize;
            assert_eq!(addr >> 2, 0x2002_0400);
        }

        #[test]
        fn pmpcfg_byte_tor_rw() {
            // TOR | R | W = 0b00001011 = 0x0B
            let byte = PMP_TOR | PMP_R | PMP_W;
            assert_eq!(byte, 0x0B);
        }

        #[test]
        fn pmpcfg_pack_two_entries() {
            // Entry 0: OFF (0x00), Entry 1: TOR|R|W (0x0B), entries 2,3: OFF.
            let cfg: [u8; 4] = [PMP_OFF, PMP_TOR | PMP_R | PMP_W, PMP_OFF, PMP_OFF];
            let packed = (cfg[0] as usize)
                | ((cfg[1] as usize) << 8)
                | ((cfg[2] as usize) << 16)
                | ((cfg[3] as usize) << 24);
            assert_eq!(packed, 0x0000_0B00);
        }
    }
}
