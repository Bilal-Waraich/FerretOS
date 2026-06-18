//! Statically allocated, capability-gated message channel.
//!
//! A [`Channel<T, CAP>`] is a fixed-capacity ring buffer with no dynamic
//! allocation — the backing storage lives inline in the struct, so a channel
//! can be a `static`.  Indices are atomic, so a channel is `Sync` and can be
//! shared between tasks through shared references.
//!
//! # Concurrency model
//!
//! The channel is single-producer / single-consumer (SPSC): one task holds the
//! [`SenderCapability`], one holds the [`ReceiverCapability`].  The producer
//! owns the write position, the consumer owns the read position, and the shared
//! length counter is updated atomically.  This matches FerretOS's single-hart,
//! capability-gated design — no two tasks ever hold the same endpoint, so there
//! is never a second producer or consumer.
//!
//! [`try_send`](Channel::try_send) and [`try_receive`](Channel::try_receive) are
//! the non-blocking core.  Blocking send/receive that parks and wakes tasks is
//! layered on top by the scheduler (issue #77).

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

/// A fixed-capacity SPSC ring buffer of `T` with `CAP` slots.
///
/// `CAP` must be greater than zero (checked at compile time).
pub struct Channel<T, const CAP: usize> {
    /// Backing storage.  Slots in `[head, head + len)` (mod `CAP`) are
    /// initialised; all others are logically uninitialised.
    buffer: UnsafeCell<[MaybeUninit<T>; CAP]>,
    /// Index of the oldest queued element (the next one to be received).
    head: AtomicUsize,
    /// Number of queued elements.  `0` = empty, `CAP` = full.
    len: AtomicUsize,
}

// SAFETY: the buffer is only ever touched through `try_send` (producer) and
// `try_receive` (consumer).  In the SPSC discipline enforced by the sender /
// receiver capability tokens, the producer is the sole writer of the tail slot
// and the consumer is the sole reader of the head slot, so the two never access
// the same slot concurrently.  `len` mediates ownership of slots and is updated
// with release/acquire ordering.  `T: Send` because values are moved between
// tasks.
unsafe impl<T: Send, const CAP: usize> Sync for Channel<T, CAP> {}

impl<T, const CAP: usize> Channel<T, CAP> {
    /// Create an empty channel.  `const`, so it can initialise a `static`.
    pub const fn new() -> Self {
        // Compile-time guard: a zero-capacity channel would divide by zero in
        // the ring-index arithmetic, so reject it when `new` is monomorphised.
        const { assert!(CAP > 0, "Channel capacity must be greater than zero") };
        Channel {
            buffer: UnsafeCell::new([const { MaybeUninit::uninit() }; CAP]),
            head: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
        }
    }

    /// Total number of slots.
    pub fn capacity(&self) -> usize {
        CAP
    }

    /// Number of queued elements.
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    /// Whether the channel holds no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the channel cannot accept another element.
    pub fn is_full(&self) -> bool {
        self.len() == CAP
    }

    /// Try to enqueue `value`.  Returns `Err(value)` (handing ownership back) if
    /// the channel is full; never blocks.
    pub fn try_send(&self, value: T) -> Result<(), T> {
        let len = self.len.load(Ordering::Acquire);
        if len == CAP {
            return Err(value);
        }
        let head = self.head.load(Ordering::Relaxed);
        let slot = (head + len) % CAP;
        // SAFETY: `slot` is in `[0, CAP)`.  `len < CAP` means this slot holds no
        // live value, so no initialised `T` is overwritten or leaked.  As the
        // sole producer we have exclusive access to the write position.
        unsafe {
            (*self.buffer.get())[slot].write(value);
        }
        // Publish the new element only after it is fully written.
        self.len.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// Try to dequeue the oldest element.  Returns `None` if the channel is
    /// empty; never blocks.
    pub fn try_receive(&self) -> Option<T> {
        let len = self.len.load(Ordering::Acquire);
        if len == 0 {
            return None;
        }
        let head = self.head.load(Ordering::Relaxed);
        // SAFETY: `len > 0` guarantees slot `head` holds a value written by an
        // earlier `try_send`.  As the sole consumer we have exclusive access;
        // we move the value out exactly once and then advance `head`, so the
        // slot is never read twice.
        let value = unsafe { (*self.buffer.get())[head].assume_init_read() };
        self.head.store((head + 1) % CAP, Ordering::Relaxed);
        // Release the slot only after the value has been moved out.
        self.len.fetch_sub(1, Ordering::Release);
        Some(value)
    }
}

impl<T, const CAP: usize> Default for Channel<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const CAP: usize> Drop for Channel<T, CAP> {
    fn drop(&mut self) {
        // Run destructors for any values still queued.  (A `static` channel
        // never drops, but a stack-allocated one in a test must not leak.)
        while self.try_receive().is_some() {}
    }
}

/// Zero-sized token granting permission to send on the channel with this `ID`.
///
/// Issued at most once per `ID` (enforced by the boot-time conflict detector,
/// issue #76), which is what makes the channel single-producer.
pub struct SenderCapability<const ID: usize>;

/// Zero-sized token granting permission to receive from the channel with this
/// `ID`.  Issued at most once per `ID`, making the channel single-consumer.
pub struct ReceiverCapability<const ID: usize>;
