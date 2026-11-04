//! FerretOS kernel library.
//!
//! All kernel subsystems live here so they can be unit-tested on the host
//! (`scripts/host_test.sh`) and model-checked by Kani.  The RISC-V entry point,
//! panic handler and boot sequence live in the thin `main.rs` binary.
//!
//! RISC-V-only code (assembly stubs, CSR access) is gated on
//! `target_arch = "riscv32"`; everything else is portable `no_std` Rust.
#![cfg_attr(not(test), no_std)]

pub mod capability;
pub mod clint;
pub mod config;
pub mod context;
pub mod generated;
pub mod ipc;
#[cfg(kani)]
pub mod kani_proofs;
pub mod memory;
pub mod scheduler;
pub mod uart;
