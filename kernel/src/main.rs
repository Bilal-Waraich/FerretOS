//! FerretOS kernel binary.
//!
//! The kernel itself lives in the `ferret` library crate (`lib.rs`).  This
//! binary only supplies the RISC-V entry point and panic handler.  On any
//! other architecture (host unit tests, Kani) it compiles to an empty `main`
//! so that `cargo test` and `cargo kani` can build the whole package.
#![cfg_attr(target_arch = "riscv32", no_std, no_main)]

#[cfg(target_arch = "riscv32")]
mod boot;
#[cfg(target_arch = "riscv32")]
mod panic;

#[cfg(not(target_arch = "riscv32"))]
fn main() {}
