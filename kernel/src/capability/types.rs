//! Zero-sized peripheral capability types.
//!
//! Each type encodes the identity of a hardware peripheral as a ZST.  Because
//! these types carry no data, they occupy no space in `.bss` or `.data` and
//! generate no machine code.  All ownership enforcement happens at compile
//! time through [`ExclusiveCapability`] and [`SharedCapability`] wrappers
//! (see `capability::wrappers`).

use core::marker::PhantomData;

/// Capability for UART peripheral number `N`.
///
/// `N` is the UART index (0-based).  On the QEMU `virt` target, N=0 maps to
/// the 16550 at `0x1000_0000`.
pub struct UartCapability<const N: usize>(PhantomData<()>);

/// Capability for GPIO pin `PIN`.
pub struct GpioCapability<const PIN: usize>(PhantomData<()>);

/// Capability for SPI controller number `N`.
pub struct SpiCapability<const N: usize>(PhantomData<()>);

/// Capability for I2C controller number `N`.
pub struct I2cCapability<const N: usize>(PhantomData<()>);

// Verify zero runtime cost at compile time.
const _: () = assert!(core::mem::size_of::<UartCapability<0>>() == 0);
const _: () = assert!(core::mem::size_of::<GpioCapability<13>>() == 0);
const _: () = assert!(core::mem::size_of::<SpiCapability<0>>() == 0);
const _: () = assert!(core::mem::size_of::<I2cCapability<0>>() == 0);
