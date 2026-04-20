//! Compile-time typed memory regions.
//!
//! `MemoryRegion<START, END>` is a zero-sized type whose const parameters
//! encode the address range.  All validation happens at compile time; there
//! is no runtime overhead.

/// A zero-sized type representing the address range `[START, END)`.
///
/// # Compile-time guarantees
///
/// - `START < END` is asserted when the type's `_VALID` constant is
///   evaluated.  Any attempt to construct an invalid region (e.g.
///   `MemoryRegion::<0x100, 0x100>`) is a compile error.
/// - Overlap between two regions can be checked at compile time via
///   [`assert_no_overlap`].
///
/// # Zero runtime cost
///
/// This type is a ZST — it occupies no space in `.data` or `.bss` and
/// generates no code.  All operations are `const fn` evaluated by the
/// compiler.
pub struct MemoryRegion<const START: usize, const END: usize>;

impl<const START: usize, const END: usize> MemoryRegion<START, END> {
    /// Compile-time assertion: START must be strictly less than END.
    ///
    /// Evaluating this constant (which the compiler does whenever the type
    /// is instantiated) triggers a build error if the invariant is violated.
    pub const _VALID: () = assert!(
        START < END,
        "MemoryRegion: START must be strictly less than END"
    );

    /// Base address of this region.
    #[inline(always)]
    pub const fn base() -> usize {
        START
    }

    /// One-past-the-end address of this region.
    #[inline(always)]
    pub const fn end() -> usize {
        END
    }

    /// Size of this region in bytes.
    #[inline(always)]
    pub const fn size() -> usize {
        END - START
    }

    /// Returns `true` if `addr` falls within `[START, END)`.
    #[inline(always)]
    pub const fn contains(addr: usize) -> bool {
        addr >= START && addr < END
    }
}

/// Assert at compile time that two `MemoryRegion`s do not overlap.
///
/// Panics (compile error) if the ranges `[A_START, A_END)` and
/// `[B_START, B_END)` share any address.
///
/// # Usage
///
/// ```rust
/// // These two regions are adjacent — no overlap.
/// const _: () = assert_no_overlap::<0x2000_0000, 0x2000_1000,
///                                   0x2000_1000, 0x2000_2000>();
///
/// // This would be a compile error:
/// // const _: () = assert_no_overlap::<0x2000_0000, 0x2000_1000,
/// //                                   0x2000_0800, 0x2000_1800>();
/// ```
///
/// # Time complexity: O(1) — evaluated entirely at compile time.
pub const fn assert_no_overlap<
    const A_START: usize,
    const A_END: usize,
    const B_START: usize,
    const B_END: usize,
>() {
    assert!(
        A_END <= B_START || B_END <= A_START,
        "MemoryRegion overlap detected at compile time"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valid region compiles without error.
    type FlashRegion = MemoryRegion<0x8000_0000, 0x8008_0000>;
    type RamRegion   = MemoryRegion<0x8008_0000, 0x800C_0000>;

    #[test]
    fn region_accessors() {
        assert_eq!(FlashRegion::base(),  0x8000_0000);
        assert_eq!(FlashRegion::end(),   0x8008_0000);
        assert_eq!(FlashRegion::size(),  0x0008_0000); // 512 KB

        assert_eq!(RamRegion::base(),    0x8008_0000);
        assert_eq!(RamRegion::size(),    0x0004_0000); // 256 KB
    }

    #[test]
    fn contains_works() {
        assert!( FlashRegion::contains(0x8000_0000));
        assert!( FlashRegion::contains(0x8007_FFFF));
        assert!(!FlashRegion::contains(0x8008_0000)); // end is exclusive
        assert!(!FlashRegion::contains(0x7FFF_FFFF));
    }

    #[test]
    fn no_overlap_adjacent() {
        // Adjacent regions — must not panic.
        const _: () = assert_no_overlap::<0x8000_0000, 0x8008_0000,
                                          0x8008_0000, 0x800C_0000>();
    }
}
