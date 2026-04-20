//! Capability system — Sprint 3.
//!
//! Hardware resources are represented as zero-sized types (ZSTs).  Ownership
//! semantics are enforced at compile time through [`ExclusiveCapability`] and
//! [`SharedCapability`] wrappers; the boot-time conflict detector validates
//! the `held_capabilities` bitmasks in each [`TaskDescriptor`] before the
//! scheduler starts.
//!
//! # Modules
//!
//! - [`types`]     — concrete peripheral ZSTs (`UartCapability`, `GpioCapability`, …)
//! - [`wrappers`]  — `ExclusiveCapability<T>` and `SharedCapability<T>`
//! - [`allocator`] — boot-time conflict detector and halt path

pub mod allocator;
pub mod types;
pub mod wrappers;

#[cfg(test)]
mod tests;

pub use allocator::check_capability_conflicts;
pub use types::{GpioCapability, I2cCapability, SpiCapability, UartCapability};
pub use wrappers::{ExclusiveCapability, SharedCapability};
