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

/// Capability to write to the kernel console through the `log` syscall.
///
/// Always held as a [`crate::capability::SharedCapability`]: any number of
/// tasks may log, but a task without it may not.  Encoded as bit
/// [`CONSOLE_CAP_BIT`] of `shared_cap_mask`; shared capabilities create no
/// CCG edges, so granting it does not change any task's priority.
pub struct ConsoleCapability(PhantomData<()>);

/// Peripheral ID of the kernel console in capability bitmasks.
pub const CONSOLE_PERIPHERAL_ID: usize = 31;

/// Bitmask for [`ConsoleCapability`].
pub const CONSOLE_CAP_BIT: u32 = 1 << CONSOLE_PERIPHERAL_ID;

// Verify zero runtime cost at compile time.
const _: () = assert!(core::mem::size_of::<UartCapability<0>>() == 0);
const _: () = assert!(core::mem::size_of::<GpioCapability<13>>() == 0);
const _: () = assert!(core::mem::size_of::<SpiCapability<0>>() == 0);
const _: () = assert!(core::mem::size_of::<I2cCapability<0>>() == 0);
const _: () = assert!(core::mem::size_of::<ConsoleCapability>() == 0);
