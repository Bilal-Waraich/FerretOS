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

pub mod region;
pub mod stack;
pub mod task;

pub use region::{MemoryRegion, assert_no_overlap};
pub use stack::Stack;
pub use task::{TaskDescriptor, TaskState, register_task, registry, task_count};
