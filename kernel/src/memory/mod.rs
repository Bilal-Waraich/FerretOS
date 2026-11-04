//! Memory safety layer — Sprint 2.
//!
//! Provides compile-time validated memory regions, ABI-aligned static stacks,
//! and the static task registry.
//!
//! # Modules
//!
//! - [`region`] — `MemoryRegion<START, END>` ZST and `assert_no_overlap`
//! - [`stack`]  — `Stack<N>` with 16-byte alignment
//! - [`task`]   — `TaskState`, `TaskDescriptor`, `TaskRegistry`
//! - [`pmp`]    — RISC-V PMP configuration (per-task isolation with feature = "pmp")

pub mod pmp;
pub mod region;
pub mod stack;
pub mod task;

pub use pmp::{activate_task_region, configure_pmp};
pub use region::{MemoryRegion, assert_no_overlap};
pub use stack::Stack;
pub use task::{BlockReason, TaskDescriptor, TaskMode, TaskState, register_task, registry, task_count};
