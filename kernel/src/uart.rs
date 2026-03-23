//! Minimal 16550-compatible UART driver for the QEMU `virt` board.
//!
//! Exposes blocking byte and string transmit helpers used by the boot
//! diagnostics and panic handler.  No receive path; no interrupt-driven TX.

/// THR/LSR base address on QEMU virt (`0x1000_0000` per device tree).
const UART_BASE: usize = 0x1000_0000;
/// Transmit Holding Register — write one byte, DLAB=0.
const UART_THR: usize = 0x00;

/// Transmit a single byte by writing directly to the THR.
///
/// # Safety
///
/// Caller must ensure no other hart or DMA channel is concurrently writing
/// to this UART.  In FerretOS's single-hart, pre-scheduler boot path this
/// invariant is always satisfied, so the safe wrappers below do not need to
/// be marked `unsafe`.
#[inline]
pub unsafe fn uart_putchar(c: u8) {
    // `write_volatile` prevents the compiler from eliding the store; MMIO
    // writes have side effects that are invisible to the optimizer.
    let thr = (UART_BASE + UART_THR) as *mut u8;
    thr.write_volatile(c);
}

/// Write every byte of `s` to the UART in order.
///
/// Non-ASCII bytes are forwarded unchanged.
pub fn uart_puts(s: &str) {
    for byte in s.bytes() {
        // SAFETY: single-hart, no concurrent UART access during boot / panic.
        unsafe { uart_putchar(byte) };
    }
}

/// Print `val` as a zero-padded `0x`-prefixed hexadecimal string.
///
/// Width is always `2 * size_of::<usize>()` digits (8 on riscv32).
pub fn uart_print_hex(val: usize) {
    const HEX: &[u8] = b"0123456789abcdef";
    uart_puts("0x");
    // Walk from MSN to LSN so output reads left-to-right.
    let bits = core::mem::size_of::<usize>() * 8;
    let mut i = bits;
    while i > 0 {
        i -= 4;
        // SAFETY: nibble is always 0–15, index is in-bounds.
        unsafe { uart_putchar(HEX[(val >> i) & 0xF]) };
    }
}

/// Print `val` in decimal without leading zeros.
///
/// Digits are extracted into a 20-byte stack buffer (sufficient for u64) and
/// printed MSB-first.
pub fn uart_print_usize(val: usize) {
    if val == 0 {
        uart_puts("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut pos = buf.len();
    while n > 0 {
        pos -= 1;
        buf[pos] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // SAFETY: buf[pos..] contains only ASCII digit bytes 0x30–0x39.
    let s = unsafe { core::str::from_utf8_unchecked(&buf[pos..]) };
    uart_puts(s);
}
