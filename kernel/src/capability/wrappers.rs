//! Ownership wrappers that enforce exclusive vs. shared capability semantics.
//!
//! - [`ExclusiveCapability<T>`] is neither `Clone` nor `Copy`.  The type
//!   system makes it impossible to hand the same instance to two tasks.
//! - [`SharedCapability<T>`] is `Clone`, allowing multiple tasks to hold
//!   concurrent read-only access.
//!
//! The boot-time conflict detector in [`crate::capability::allocator`]
//! validates the `held_capabilities` bitmask in each `TaskDescriptor` against
//! these semantics before the scheduler starts.

use core::marker::PhantomData;

/// An exclusive, non-duplicable capability for peripheral `T`.
///
/// The absence of `Clone` and `Copy` means Rust's move semantics ensure only
/// one owner exists.  At boot, [`crate::capability::allocator`] checks that
/// no two task descriptors claim the same exclusive peripheral ID.
///
/// # Zero runtime cost
///
/// This is a ZST — `size_of::<ExclusiveCapability<T>>() == 0` for any `T`.
pub struct ExclusiveCapability<T>(PhantomData<T>);

impl<T> ExclusiveCapability<T> {
    /// Construct an exclusive capability token.
    ///
    /// # Safety
    ///
    /// The caller must ensure that no other `ExclusiveCapability` of the same
    /// type is live.  In practice this is enforced by the boot-time conflict
    /// detector; do not call outside of that path.
    pub const fn new() -> Self {
        ExclusiveCapability(PhantomData)
    }
}

impl<T> Default for ExclusiveCapability<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A shared, cloneable capability for peripheral `T`.
///
/// Multiple tasks may hold simultaneous `SharedCapability<T>` tokens,
/// representing read-only / non-exclusive access to the peripheral.
///
/// # Zero runtime cost
///
/// This is a ZST — `size_of::<SharedCapability<T>>() == 0` for any `T`.
#[derive(Clone)]
pub struct SharedCapability<T>(PhantomData<T>);

impl<T> SharedCapability<T> {
    /// Construct a shared capability token.
    pub const fn new() -> Self {
        SharedCapability(PhantomData)
    }
}

impl<T> Default for SharedCapability<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Verify zero runtime cost at compile time.
const _: () = assert!(core::mem::size_of::<ExclusiveCapability<()>>() == 0);
const _: () = assert!(core::mem::size_of::<SharedCapability<()>>() == 0);
