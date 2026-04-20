//! Statically allocated, ABI-aligned task stacks.
//!
//! Each task owns a `Stack<N>` that lives in `.bss`.  The size `N` comes from
//! the task declaration (OML `stack_size` field in Sprint 5).

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
    /// Create a zero-initialised stack.
    ///
    /// # Panics (compile time)
    ///
    /// Panics at compile time if `N` is not a multiple of 16, since an
    /// unaligned stack top would violate the RISC-V ABI.
    pub const fn new() -> Self {
        assert!(N % 16 == 0, "Stack size N must be a multiple of 16 bytes");
        Stack([0u8; N])
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
}
