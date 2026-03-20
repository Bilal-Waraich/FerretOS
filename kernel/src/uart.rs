//! uart.rs — Minimal 16550-compatible UART driver for the QEMU virt board.
//!
//! # Why do we need this?
//!
//! Before we have a scheduler, a filesystem, or even a proper allocator, we
//! need *some* way to see what the kernel is doing.  The QEMU virt machine
//! exposes a 16550-compatible UART at 0x10000000 whose output goes straight
//! to your terminal.  This module gives us `uart_puts` and friends so we can
//! print debug strings from anywhere in the kernel without pulling in `std`.
//!
//! # What is the 16550?
//!
//! The NS16550A is a UART chip from the 1980s that became the de-facto
//! standard for serial ports on PC-compatible hardware.  QEMU emulates it
//! faithfully.  We only use two of its registers:
//!
//! - **THR** (Transmit Holding Register) at base+0: writing a byte here sends
//!   it out the serial line.  QEMU forwards it to stdout immediately.
//! - **LSR** (Line Status Register) at base+5: bit 5 ("THRE") is set when the
//!   transmitter is ready for another byte.  Polling this prevents us from
//!   dropping characters at high baud rates.
//!
//! In Sprint 0 we skip the LSR poll (QEMU's FIFO never actually fills up at
//! kernel-boot speeds), but the comment is here so the code is easy to harden
//! later.
//!
//! # Why `write_volatile`?
//!
//! The Rust compiler is allowed to reorder, merge, or eliminate memory
//! accesses that it believes have no observable effect.  MMIO registers are
//! *not* ordinary memory — writing the same value twice to THR sends two
//! characters, not one.  `write_volatile` tells the compiler "this write has
//! a side effect you cannot see; do not touch it."  Without it, an optimising
//! build could silently drop our debug output.

/// Base address of the UART on the QEMU virt machine.
///
/// This matches QEMU's hard-coded device tree entry for the virt board.  If
/// you ever port FerretOS to real hardware, this is the first thing to change.
/// Note: 0x1000_0000 = 268,435,456 — fits comfortably in a 32-bit usize.
const UART_BASE: usize = 0x1000_0000;

/// Offset of the Transmit Holding Register within the 16550 register map.
/// When DLAB=0 (the default after reset), a write here sends one byte.
const UART_THR: usize = 0x00;

/// Send a single byte out the UART.
///
/// # Safety
///
/// This function performs a raw pointer write to a memory-mapped hardware
/// register.  It is safe to call as long as `UART_BASE` is correct for the
/// running machine and we are the only code writing to that address.  In our
/// single-hart, no-RTOS kernel that is always true, so the internal callers
/// (`uart_puts`, etc.) are not marked `unsafe` even though they ultimately
/// invoke this function.
#[inline]
pub unsafe fn uart_putchar(c: u8) {
    // Cast the register address to a raw mutable pointer and write through it.
    // The `write_volatile` call compiles to a single `sb` instruction on
    // RISC-V; there is no caching or buffering between us and the device.
    let thr = (UART_BASE + UART_THR) as *mut u8;
    thr.write_volatile(c);
}

/// Write a UTF-8 string slice to the UART, one byte at a time.
///
/// Non-ASCII bytes are passed through unchanged.  If you ever need true UTF-8
/// handling (e.g. for locale-aware error messages), that can be layered on top.
pub fn uart_puts(s: &str) {
    for byte in s.bytes() {
        // SAFETY: UART_BASE is correct for the QEMU virt board and we are the
        // sole owner of the UART hardware in this single-tasking Sprint 0 kernel.
        unsafe { uart_putchar(byte) };
    }
}

/// Print a `usize` value as a `0x`-prefixed hexadecimal string.
///
/// Useful for dumping addresses and register values in panic messages and
/// boot diagnostics without needing an allocator or `format!`.
pub fn uart_print_hex(val: usize) {
    // Hex digits look-up table — avoids a branch-heavy nibble-to-char conversion.
    const HEX: &[u8] = b"0123456789abcdef";

    uart_puts("0x");

    // On riscv32 a usize is 32 bits = 8 hex digits.  We walk from the most
    // significant nibble down so the output reads left-to-right.
    let bits = core::mem::size_of::<usize>() * 8; // 32 on riscv32
    let mut i = bits;
    while i > 0 {
        i -= 4;
        let nibble = (val >> i) & 0xF;
        // SAFETY: same as uart_putchar above.
        unsafe { uart_putchar(HEX[nibble]) };
    }
}

/// Print a `usize` value in decimal, without leading zeros.
///
/// We can't use `format!` because that requires an allocator.  Instead we
/// extract digits right-to-left into a small stack buffer and then print them
/// in the correct order.  The buffer needs at most 10 digits for a 32-bit
/// value (4,294,967,295).
pub fn uart_print_usize(val: usize) {
    if val == 0 {
        uart_puts("0");
        return;
    }

    // 20 bytes is enough for a 64-bit decimal value, so it covers 32-bit too.
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut pos = buf.len();

    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }

    // `pos` now points at the first (most-significant) digit.
    // SAFETY: buf[pos..] contains only ASCII digits, so it is valid UTF-8.
    let s = unsafe { core::str::from_utf8_unchecked(&buf[pos..]) };
    uart_puts(s);
}
