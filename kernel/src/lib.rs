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

// The build-time CCG generator (kernel/build/ccg_gen.rs) is host code used by
// build.rs; compiling it into the host test build lets its unit tests run and
// lets them cross-check it against the in-kernel algorithm.
#[cfg(test)]
#[path = "../build/ccg_gen.rs"]
mod ccg_gen;
pub mod sys;
pub mod syscall;

/// Halt the hart permanently after a fatal error.
///
/// Used by boot-time and runtime fatal paths (CLAUDE.md §6.5) once the
/// diagnostic has been written to UART.  Under `cfg(test)` it panics instead,
/// so host tests can assert that a fatal path was taken.
pub fn halt() -> ! {
    #[cfg(test)]
    panic!("kernel halted");
    #[cfg(not(test))]
    loop {
        // SAFETY: `wfi` only pauses the hart until an interrupt is pending;
        // it has no other architectural effect.
        #[cfg(target_arch = "riscv32")]
        unsafe {
            core::arch::asm!("wfi")
        };
        #[cfg(not(target_arch = "riscv32"))]
        core::hint::spin_loop();
    }
}
