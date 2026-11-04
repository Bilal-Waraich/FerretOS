//! Task-side syscall wrappers.
//!
//! Task code calls these instead of touching kernel state or MMIO directly.
//! Each wrapper issues one `ecall`; the kernel side is [`crate::syscall`].
//! On non-RISC-V builds (host tests, Kani) the wrappers are inert stubs.

use crate::syscall::{SYS_LOG, SYS_NOW, SYS_SLEEP, SYS_YIELD};

#[inline(always)]
fn ecall(n: usize, a0: usize, a1: usize) -> usize {
    #[cfg(target_arch = "riscv32")]
    {
        let ret: usize;
        // SAFETY: `ecall` traps into the kernel, which saves and restores the
        // full register file, so only `a0` (the return value) changes.  Memory
        // may be modified by other tasks while this one is switched out, so
        // the asm is not marked `nomem`.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a0 => ret,
                in("a1") a1,
                in("a7") n,
                options(nostack),
            );
        }
        ret
    }
    #[cfg(not(target_arch = "riscv32"))]
    {
        let _ = (n, a1);
        a0
    }
}

/// Give up the CPU to the next ready task of equal or higher priority.
pub fn yield_now() {
    ecall(SYS_YIELD, 0, 0);
}

/// Block the calling task for `ticks` timer ticks (1 tick = 1 ms).
pub fn sleep(ticks: u32) {
    ecall(SYS_SLEEP, ticks as usize, 0);
}

/// Write `bytes` to the console as one uninterrupted unit.
pub fn log(bytes: &[u8]) {
    ecall(SYS_LOG, bytes.as_ptr() as usize, bytes.len());
}

/// Return the number of timer ticks since boot.
pub fn now() -> u32 {
    ecall(SYS_NOW, 0, 0) as u32
}

/// Fixed-capacity line buffer for building log messages without allocation.
///
/// Writes past the capacity are silently truncated.
pub struct LineBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> LineBuf<N> {
    /// Create an empty buffer.
    pub const fn new() -> Self {
        LineBuf { buf: [0; N], len: 0 }
    }

    /// Append a string.
    pub fn str(&mut self, s: &str) -> &mut Self {
        for &b in s.as_bytes() {
            if self.len == N {
                break;
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        self
    }

    /// Append `v` in decimal.
    pub fn u32(&mut self, mut v: u32) -> &mut Self {
        let mut digits = [0u8; 10];
        let mut n = 0;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            if self.len == N {
                break;
            }
            self.buf[self.len] = digits[n];
            self.len += 1;
        }
        self
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Send the buffer to the console with [`log`].
    pub fn log(&self) {
        log(self.as_bytes());
    }
}

impl<const N: usize> Default for LineBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::LineBuf;

    #[test]
    fn line_buf_formats_numbers() {
        let mut b: LineBuf<32> = LineBuf::new();
        b.str("t=").u32(0).str(" ").u32(4_294_967_295);
        assert_eq!(b.as_bytes(), b"t=0 4294967295");
    }

    #[test]
    fn line_buf_truncates_at_capacity() {
        let mut b: LineBuf<4> = LineBuf::new();
        b.str("abc").u32(123);
        assert_eq!(b.as_bytes(), b"abc1");
    }
}
