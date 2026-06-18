//! Inter-task communication (IPC) — Sprint 9.
//!
//! Tasks exchange data through statically allocated, capability-gated channels.
//! A [`Channel<T, CAP>`] is a fixed-capacity ring buffer that lives in `.bss`
//! (no heap); a task may write to it only if it holds the matching
//! [`SenderCapability`], and read only if it holds the [`ReceiverCapability`].
//!
//! # Modules
//!
//! - [`channel`] — the `Channel<T, CAP>` ring buffer and the endpoint tokens.

pub mod channel;

pub use channel::{Channel, ReceiverCapability, SenderCapability};
