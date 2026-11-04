//! RISC-V PMP (Physical Memory Protection) configuration.
//!
//! PMP is what turns the type-level `MemoryRegion` guarantees into hardware
//! enforcement: a U-mode access that no PMP entry permits raises an access
//! fault.  M-mode (the kernel) is unaffected by unlocked entries.
//!
//! # Layout with `feature = "pmp"` (per-task isolation)
//!
//! | Entry | Covers | U-mode access |
//! |---|---|---|
//! | 0 | flash image (`FLASH_ORIGIN`, `FLASH_SIZE`), NAPOT | R + X |
//! | 1–2 | the **running** task's memory region (NAPOT in entry 2, or TOR with entry 1 as base) | R + W |
//! | 3–15 | off | — |
//!
//! Kernel RAM, other tasks' regions and MMIO match no entry, so U-mode cannot
//! touch them.  Entries 1–2 are rewritten by [`activate_task_region`] on
//! every context switch — two CSR writes plus one read-modify-write of
//! `pmpcfg0`.
//!
//! # Layout without `feature = "pmp"` (no isolation)
//!
//! Entry 15 grants U-mode R + W + X over the whole address space; without
//! *some* matching entry, U-mode could not execute at all.
//!
//! # Encoding
//!
//! `pmpcfgN` holds one byte per entry: `[L | 00 | A[1:0] | X | W | R]`, with
//! `A` = 00 OFF, 01 TOR, 11 NAPOT.  `pmpaddr` holds a physical address >> 2.
//! NAPOT encodes a naturally aligned power-of-two region of size `S ≥ 8` as
//! `(base | (S/2 - 1)) >> 2`.  A region that is not NAPOT-compatible falls
//! back to TOR, which needs two entries: `pmpaddr[i-1] ≤ addr < pmpaddr[i]`.

use crate::memory::task::{TaskDescriptor, TaskMode};

/// `pmpcfg` permission bit: readable.
pub const PMP_R: u8 = 1 << 0;
/// `pmpcfg` permission bit: writable.
pub const PMP_W: u8 = 1 << 1;
/// `pmpcfg` permission bit: executable.
pub const PMP_X: u8 = 1 << 2;
/// `pmpcfg` address mode: Top Of Range.
pub const PMP_TOR: u8 = 0b01 << 3;
/// `pmpcfg` address mode: naturally aligned power of two.
pub const PMP_NAPOT: u8 = 0b11 << 3;

/// Entry covering the flash image (feature `pmp`).
pub const FLASH_ENTRY: usize = 0;
/// First of the two entries used for the running task's region.
pub const TASK_ENTRY_LO: usize = 1;
/// Second entry used for the running task's region (holds the mode byte).
pub const TASK_ENTRY_HI: usize = 2;
/// Entry used for the permissive default grant (no `pmp` feature).
pub const DEFAULT_GRANT_ENTRY: usize = 15;

/// How a region is programmed into the two task entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryPair {
    /// `pmpaddr` for entry `TASK_ENTRY_LO`.
    pub addr_lo: usize,
    /// `pmpaddr` for entry `TASK_ENTRY_HI`.
    pub addr_hi: usize,
    /// `pmpcfg` byte for entry `TASK_ENTRY_LO` (always OFF).
    pub cfg_lo: u8,
    /// `pmpcfg` byte for entry `TASK_ENTRY_HI`.
    pub cfg_hi: u8,
}

/// Whether `[start, end)` can be expressed as a single NAPOT entry.
pub fn is_napot(start: usize, end: usize) -> bool {
    let Some(size) = end.checked_sub(start) else { return false };
    size >= 8 && size.is_power_of_two() && start % size == 0
}

/// `pmpaddr` value for a NAPOT region.  Caller must ensure [`is_napot`].
pub fn napot_addr(start: usize, end: usize) -> usize {
    let size = end - start;
    (start | (size / 2 - 1)) >> 2
}

/// Encode region `[start, end)` with permissions `perms` into the two task
/// entries: NAPOT in the high entry when possible, otherwise TOR.
///
/// An empty or inverted region yields both entries OFF (no access).
pub fn encode_region(start: usize, end: usize, perms: u8) -> EntryPair {
    if end <= start {
        return EntryPair { addr_lo: 0, addr_hi: 0, cfg_lo: 0, cfg_hi: 0 };
    }
    if is_napot(start, end) {
        EntryPair { addr_lo: 0, addr_hi: napot_addr(start, end), cfg_lo: 0, cfg_hi: PMP_NAPOT | perms }
    } else {
        EntryPair { addr_lo: start >> 2, addr_hi: end >> 2, cfg_lo: 0, cfg_hi: PMP_TOR | perms }
    }
}

/// The entries the scheduler programs when `task` is about to run.
///
/// U-mode tasks get R + W on their memory region.  M-mode tasks are not
/// constrained by PMP, so their entries are turned off.
pub fn task_entries(task: &TaskDescriptor) -> EntryPair {
    match task.mode {
        TaskMode::User => encode_region(task.memory_start, task.memory_end, PMP_R | PMP_W),
        TaskMode::Machine => encode_region(0, 0, 0),
    }
}

/// Configure PMP on the boot path, before any U-mode code runs.
///
/// With `feature = "pmp"`: flash R + X in entry 0, everything else off until
/// [`activate_task_region`] runs for the first task.  Without it: the
/// permissive whole-address-space grant in entry 15.
pub fn configure_pmp() {
    #[cfg(feature = "pmp")]
    {
        use crate::config::{FLASH_ORIGIN, FLASH_SIZE};
        const _: () = assert!(FLASH_SIZE.is_power_of_two() && FLASH_ORIGIN % FLASH_SIZE == 0);
        // SAFETY: M-mode boot path, single hart, before any U-mode code runs.
        unsafe {
            csr::write_pmpcfg0(0);
            csr::write_pmpaddr(FLASH_ENTRY, napot_addr(FLASH_ORIGIN, FLASH_ORIGIN + FLASH_SIZE));
            csr::write_cfg_byte(FLASH_ENTRY, PMP_NAPOT | PMP_R | PMP_X);
        }
        #[cfg(not(test))]
        crate::uart::uart_puts("PMP: per-task isolation enabled (flash R+X, running task R+W).\n");
    }
    #[cfg(not(feature = "pmp"))]
    {
        // SAFETY: as above.  NAPOT with an all-ones address covers the entire
        // 34-bit physical address space.
        unsafe {
            csr::write_pmpaddr(DEFAULT_GRANT_ENTRY, usize::MAX);
            csr::write_cfg_byte(DEFAULT_GRANT_ENTRY, PMP_NAPOT | PMP_R | PMP_W | PMP_X);
        }
    }
}

/// Point the task entries at `task`'s memory region (feature `pmp`).
///
/// Called by the scheduler on every dispatch, before returning to the task.
/// A no-op without the `pmp` feature.
pub fn activate_task_region(task: &TaskDescriptor) {
    #[cfg(feature = "pmp")]
    {
        let e = task_entries(task);
        // SAFETY: trap path in M-mode with interrupts disabled; the entries
        // only take effect for U-mode after the following mret.  The high
        // entry is disabled first so no transient TOR/NAPOT combination of
        // old and new addresses is ever live.
        unsafe {
            csr::write_cfg_byte(TASK_ENTRY_HI, 0);
            csr::write_pmpaddr(TASK_ENTRY_LO, e.addr_lo);
            csr::write_pmpaddr(TASK_ENTRY_HI, e.addr_hi);
            csr::write_cfg_byte(TASK_ENTRY_LO, e.cfg_lo);
            csr::write_cfg_byte(TASK_ENTRY_HI, e.cfg_hi);
        }
    }
    #[cfg(not(feature = "pmp"))]
    let _ = task;
}

/// Raw CSR access.  RV32 requires the CSR number as an immediate, so each
/// register gets its own instruction; on non-RISC-V hosts these are no-ops.
mod csr {
    /// Write `pmpaddr<idx>`.
    ///
    /// # Safety
    ///
    /// M-mode only, single hart; changes U-mode memory permissions.
    pub unsafe fn write_pmpaddr(idx: usize, val: usize) {
        #[cfg(target_arch = "riscv32")]
        macro_rules! w {
            ($csr:literal) => {
                // SAFETY: delegated to the caller.
                unsafe { core::arch::asm!(concat!("csrw ", $csr, ", {v}"), v = in(reg) val, options(nomem, nostack)) }
            };
        }
        #[cfg(target_arch = "riscv32")]
        match idx {
            0 => w!("pmpaddr0"),
            1 => w!("pmpaddr1"),
            2 => w!("pmpaddr2"),
            15 => w!("pmpaddr15"),
            _ => {}
        }
        #[cfg(not(target_arch = "riscv32"))]
        let _ = (idx, val);
    }

    /// Overwrite `pmpcfg0` (entries 0–3).
    ///
    /// # Safety
    ///
    /// As [`write_pmpaddr`].
    #[cfg_attr(not(feature = "pmp"), allow(dead_code))]
    pub unsafe fn write_pmpcfg0(val: usize) {
        // SAFETY: delegated to the caller.
        #[cfg(target_arch = "riscv32")]
        unsafe {
            core::arch::asm!("csrw pmpcfg0, {v}", v = in(reg) val, options(nomem, nostack))
        };
        #[cfg(not(target_arch = "riscv32"))]
        let _ = val;
    }

    /// Set the configuration byte of entry `idx` (0–3 or 15), leaving the
    /// other bytes of its `pmpcfg` register unchanged.
    ///
    /// # Safety
    ///
    /// As [`write_pmpaddr`].
    pub unsafe fn write_cfg_byte(idx: usize, byte: u8) {
        let shift = (idx % 4) * 8;
        let clear = 0xFFusize << shift;
        let set = (byte as usize) << shift;
        #[cfg(target_arch = "riscv32")]
        macro_rules! rmw {
            ($csr:literal) => {
                // SAFETY: delegated to the caller.
                unsafe {
                    core::arch::asm!(
                        concat!("csrc ", $csr, ", {c}"),
                        concat!("csrs ", $csr, ", {s}"),
                        c = in(reg) clear,
                        s = in(reg) set,
                        options(nomem, nostack),
                    )
                }
            };
        }
        #[cfg(target_arch = "riscv32")]
        match idx / 4 {
            0 => rmw!("pmpcfg0"),
            3 => rmw!("pmpcfg3"),
            _ => {}
        }
        #[cfg(not(target_arch = "riscv32"))]
        let _ = (clear, set);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn napot_encoding_of_4k_region() {
        // (0x800B_8000 | 0x7FF) >> 2: nine trailing ones encode 2^(9+3) = 4 KB.
        assert!(is_napot(0x800B_8000, 0x800B_9000));
        assert_eq!(napot_addr(0x800B_8000, 0x800B_9000), 0x2002_E1FF);
    }

    #[test]
    fn napot_encoding_of_flash() {
        assert!(is_napot(0x8000_0000, 0x8008_0000));
        assert_eq!(napot_addr(0x8000_0000, 0x8008_0000), 0x2000_FFFF);
    }

    #[test]
    fn misaligned_or_odd_sized_region_falls_back_to_tor() {
        assert!(!is_napot(0x800B_8800, 0x800B_9800)); // 4 KB but not 4 KB aligned
        assert!(!is_napot(0x800B_8000, 0x800B_8C00)); // 3 KB
        let e = encode_region(0x800B_8000, 0x800B_8C00, PMP_R | PMP_W);
        assert_eq!(e.addr_lo, 0x800B_8000 >> 2);
        assert_eq!(e.addr_hi, 0x800B_8C00 >> 2);
        assert_eq!(e.cfg_lo, 0);
        assert_eq!(e.cfg_hi, PMP_TOR | PMP_R | PMP_W);
    }

    #[test]
    fn napot_region_uses_high_entry_only() {
        let e = encode_region(0x800B_9000, 0x800B_A000, PMP_R | PMP_W);
        assert_eq!(e.cfg_lo, 0);
        assert_eq!(e.cfg_hi, PMP_NAPOT | PMP_R | PMP_W);
        assert_eq!(e.addr_hi, napot_addr(0x800B_9000, 0x800B_A000));
    }

    #[test]
    fn empty_region_grants_nothing() {
        assert_eq!(encode_region(0, 0, PMP_R | PMP_W), EntryPair { addr_lo: 0, addr_hi: 0, cfg_lo: 0, cfg_hi: 0 });
        assert_eq!(encode_region(0x2000, 0x1000, PMP_R).cfg_hi, 0);
    }

    #[test]
    fn user_task_gets_rw_without_execute_and_machine_task_gets_nothing() {
        let user = TaskDescriptor::new(0, 1, 0x800B_8000, 4096, 0x800B_8000, 0x800B_9000);
        let e = task_entries(&user);
        assert_eq!(e.cfg_hi & (PMP_R | PMP_W | PMP_X), PMP_R | PMP_W);
        let kernel = user.with_mode(TaskMode::Machine);
        assert_eq!(task_entries(&kernel).cfg_hi, 0);
    }

    #[test]
    fn mode_bits_match_privileged_spec() {
        assert_eq!(PMP_TOR, 0x08);
        assert_eq!(PMP_NAPOT, 0x18);
        assert_eq!(PMP_NAPOT | PMP_R | PMP_W | PMP_X, 0x1F);
    }
}
