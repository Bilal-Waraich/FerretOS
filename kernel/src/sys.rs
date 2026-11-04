//! Task-side syscall wrappers.
//!
//! Task code calls these instead of touching kernel state or MMIO directly.
//! Each wrapper issues one `ecall`; the kernel side is [`crate::syscall`].
//! On non-RISC-V builds (host tests, Kani) the wrappers are inert stubs.

use crate::ipc::{ReceiverCapability, SenderCapability};
use crate::syscall::{
    EAGAIN, EFAULT, EINVAL, ENOSYS, EPERM, FLAG_NONBLOCK, SYS_LOG, SYS_NOW, SYS_RECEIVE,
    SYS_SEND, SYS_SLEEP, SYS_YIELD,
};

/// Error returned by a syscall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysError {
    /// Unknown syscall number.
    NoSys,
    /// The caller lacks the required capability.
    Permission,
    /// A buffer argument is not accessible by the caller.
    Fault,
    /// The operation would block (non-blocking variant only).
    WouldBlock,
    /// An argument is out of range.
    Invalid,
}

/// Decode a raw syscall return value.
fn decode(ret: usize) -> Result<usize, SysError> {
    match ret {
        ENOSYS => Err(SysError::NoSys),
        EPERM => Err(SysError::Permission),
        EFAULT => Err(SysError::Fault),
        EAGAIN => Err(SysError::WouldBlock),
        EINVAL => Err(SysError::Invalid),
        n => Ok(n),
    }
}

#[inline(always)]
fn ecall(n: usize, a0: usize, a1: usize) -> usize {
    ecall4(n, a0, a1, 0, 0)
}

#[inline(always)]
fn ecall4(n: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> usize {
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
                in("a2") a2,
                in("a3") a3,
                in("a7") n,
                options(nostack),
            );
        }
        ret
    }
    #[cfg(not(target_arch = "riscv32"))]
    {
        let _ = (n, a1, a2, a3);
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
///
/// Requires the console capability.  Returns the number of bytes written
/// (messages longer than `LOG_MAX_LEN` are truncated).
pub fn log(bytes: &[u8]) -> Result<usize, SysError> {
    log_raw(bytes.as_ptr() as usize, bytes.len())
}

/// `log` taking a raw address and length.
///
/// Safe to call with any values: the kernel validates the buffer and returns
/// [`SysError::Fault`] if the caller could not read it itself.
pub fn log_raw(addr: usize, len: usize) -> Result<usize, SysError> {
    decode(ecall(SYS_LOG, addr, len))
}

/// Return the number of timer ticks since boot.
pub fn now() -> u32 {
    ecall(SYS_NOW, 0, 0) as u32
}

impl<const ID: usize> SenderCapability<ID> {
    /// Send `bytes` (at most `ipc::kernel::MSG_MAX`) on channel `ID`,
    /// blocking while the channel is full.
    ///
    /// The token is the type-level permission; the kernel independently
    /// checks the task's boot-validated `sends_mask`.
    pub fn send(&self, bytes: &[u8]) -> Result<(), SysError> {
        decode(ecall4(SYS_SEND, ID, bytes.as_ptr() as usize, bytes.len(), 0)).map(|_| ())
    }

    /// As [`send`](Self::send), but returns [`SysError::WouldBlock`] when full.
    pub fn try_send(&self, bytes: &[u8]) -> Result<(), SysError> {
        decode(ecall4(SYS_SEND, ID, bytes.as_ptr() as usize, bytes.len(), FLAG_NONBLOCK)).map(|_| ())
    }
}

impl<const ID: usize> ReceiverCapability<ID> {
    /// Receive one message from channel `ID` into `buf`, blocking while the
    /// channel is empty.  Returns the message length; a message longer than
    /// `buf` is truncated.
    pub fn receive(&self, buf: &mut [u8]) -> Result<usize, SysError> {
        decode(ecall4(SYS_RECEIVE, ID, buf.as_mut_ptr() as usize, buf.len(), 0))
    }

    /// As [`receive`](Self::receive), but returns [`SysError::WouldBlock`]
    /// when the channel is empty.
    pub fn try_receive(&self, buf: &mut [u8]) -> Result<usize, SysError> {
        decode(ecall4(SYS_RECEIVE, ID, buf.as_mut_ptr() as usize, buf.len(), FLAG_NONBLOCK))
    }
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

    /// Append `v` as `0x`-prefixed, zero-padded 8-digit hexadecimal
    /// (the same format the kernel's UART diagnostics use).
    pub fn hex(&mut self, v: u32) -> &mut Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        self.str("0x");
        for shift in (0..8).rev() {
            if self.len == N {
                break;
            }
            self.buf[self.len] = DIGITS[((v >> (shift * 4)) & 0xF) as usize];
            self.len += 1;
        }
        self
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Send the buffer to the console with [`log`].
    pub fn log(&self) -> Result<usize, SysError> {
        log(self.as_bytes())
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
    fn line_buf_formats_hex() {
        let mut b: LineBuf<32> = LineBuf::new();
        b.hex(0x8008_0780).str(" ").hex(0);
        assert_eq!(b.as_bytes(), b"0x80080780 0x00000000");
    }

    #[test]
    fn line_buf_truncates_at_capacity() {
        let mut b: LineBuf<4> = LineBuf::new();
        b.str("abc").u32(123);
        assert_eq!(b.as_bytes(), b"abc1");
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    #[test]
    fn decode_maps_error_codes() {
        assert_eq!(decode(ENOSYS), Err(SysError::NoSys));
        assert_eq!(decode(EPERM), Err(SysError::Permission));
        assert_eq!(decode(EFAULT), Err(SysError::Fault));
        assert_eq!(decode(42), Ok(42));
    }
}
