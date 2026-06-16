//! Statically allocated, ABI-aligned task stacks.
//!
//! Each task owns a `Stack<N>` that lives in `.bss`.  The size `N` comes from
//! the task declaration (OML `stack_size` field in Sprint 5).

/// Magic value written at the lowest address of every stack buffer.
///
/// If a stack overflows downward it overwrites this sentinel.  The timer ISR
/// calls [`Stack::check_canary`] on every tick so that corruption is caught
/// within one scheduling quantum rather than silently propagating.
pub const STACK_CANARY: u32 = 0xDEAD_C0DE;

/// A statically allocated stack of `N` bytes, aligned to 16 bytes.
///
/// # ABI requirement
///
/// The RISC-V calling convention requires the stack pointer to be 16-byte
/// aligned at all function call boundaries.  `#[repr(align(16))]` satisfies
/// this without any runtime adjustment.
///
/// # Layout
///
/// Stacks grow downward on RISC-V: the initial stack pointer points to the
/// *top* of the buffer (one past the last byte), and the hardware decrements
/// `sp` as frames are pushed.  Use [`Stack::top_ptr`] to obtain this address.
///
/// # Size parameter
///
/// `N` must be a multiple of 16 (to keep the top pointer 16-byte aligned).
/// The const assertion in [`Stack::new`] enforces this at compile time.
#[repr(align(16))]
pub struct Stack<const N: usize>([u8; N]);

impl<const N: usize> Stack<N> {
    /// Create a zero-initialised stack with the canary sentinel written at
    /// the lowest address (offset 0).
    ///
    /// # Panics (compile time)
    ///
    /// Panics at compile time if `N` is not a multiple of 16, since an
    /// unaligned stack top would violate the RISC-V ABI.
    ///
    /// # Panics (compile time)
    ///
    /// Panics at compile time if `N < 4`, since the canary occupies the first
    /// 4 bytes and at least one byte of usable stack must remain.
    pub const fn new() -> Self {
        assert!(N % 16 == 0, "Stack size N must be a multiple of 16 bytes");
        assert!(N >= 16, "Stack size N must be at least 16 bytes");
        let mut buf = [0u8; N];
        // Write STACK_CANARY in little-endian at the lowest address.
        buf[0] = (STACK_CANARY & 0xFF) as u8;
        buf[1] = ((STACK_CANARY >> 8) & 0xFF) as u8;
        buf[2] = ((STACK_CANARY >> 16) & 0xFF) as u8;
        buf[3] = ((STACK_CANARY >> 24) & 0xFF) as u8;
        Stack(buf)
    }
}

impl<const N: usize> Default for Stack<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Stack<N> {

    /// Return a pointer to the top of the stack (one past the last byte).
    ///
    /// On RISC-V, `sp` is initialised to this address; the CPU decrements it
    /// before each push.  The address is 16-byte aligned because `N % 16 == 0`
    /// and the struct itself is `align(16)`.
    ///
    /// # Safety
    ///
    /// The returned pointer is valid as long as `self` is live.  It must only
    /// be installed as the stack pointer of a task that will not outlive `self`.
    pub fn top_ptr(&mut self) -> *mut u8 {
        // SAFETY: adding N to the base pointer lands one-past-the-end of the
        // array, which is a valid (non-dereferenceable) pointer per Rust rules.
        unsafe { self.0.as_mut_ptr().add(N) }
    }

    /// Return the size of the stack in bytes.
    #[inline(always)]
    pub const fn size() -> usize {
        N
    }

    /// Return `true` if the canary sentinel at offset 0 is intact.
    ///
    /// A corrupted canary means the stack has grown past its bottom — the task
    /// has overflowed into adjacent memory.  Call this from the timer ISR on
    /// every tick to catch overflow within one scheduling quantum.
    ///
    /// # Safety
    ///
    /// Reads the first 4 bytes of the buffer via a volatile `u32` load to
    /// prevent the compiler from optimising away the check.
    pub fn check_canary(&self) -> bool {
        // SAFETY: buf is at least 16 bytes (enforced by new()), so reading a
        // u32 at offset 0 is within bounds and correctly aligned (Stack is
        // align(16)).  The volatile read prevents the compiler from caching or
        // eliminating this check.
        let value = unsafe {
            core::ptr::read_volatile(self.0.as_ptr() as *const u32)
        };
        value == STACK_CANARY
    }
}
