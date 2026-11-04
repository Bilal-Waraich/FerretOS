//! Kernel-owned channel table used by the `send` / `receive` syscalls.
//!
//! U-mode tasks cannot reach kernel memory (PMP), so channels live here and
//! tasks exchange data by value: `send` copies a message out of the caller's
//! memory into the channel, `receive` copies one into the caller's buffer.
//! Each slot is a [`Channel`] of fixed-size [`Message`]s, so the ring buffer
//! (and its Kani harnesses) is the same code tasks would use in-process.

use super::channel::Channel;
use crate::config::MAX_CHANNELS;

/// Largest message payload in bytes.
pub const MSG_MAX: usize = 16;

/// Messages each channel can buffer before `send` blocks.
pub const CHANNEL_CAPACITY: usize = 8;

/// One queued message: up to [`MSG_MAX`] bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    len: u8,
    data: [u8; MSG_MAX],
}

impl Message {
    /// Build a message from `bytes`.  Returns `None` if it exceeds [`MSG_MAX`].
    pub fn from_bytes(bytes: &[u8]) -> Option<Message> {
        if bytes.len() > MSG_MAX {
            return None;
        }
        let mut data = [0u8; MSG_MAX];
        data[..bytes.len()].copy_from_slice(bytes);
        Some(Message { len: bytes.len() as u8, data })
    }

    /// The payload.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

/// The kernel's channels, indexed by channel ID.
static CHANNELS: [Channel<Message, CHANNEL_CAPACITY>; MAX_CHANNELS] =
    [const { Channel::new() }; MAX_CHANNELS];

/// Channel `id`, or `None` if `id >= MAX_CHANNELS`.
pub fn channel(id: usize) -> Option<&'static Channel<Message, CHANNEL_CAPACITY>> {
    CHANNELS.get(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_bytes() {
        let m = Message::from_bytes(b"abc").unwrap();
        assert_eq!(m.bytes(), b"abc");
        assert_eq!(Message::from_bytes(&[0u8; MSG_MAX]).unwrap().bytes().len(), MSG_MAX);
    }

    #[test]
    fn oversized_message_is_rejected() {
        assert!(Message::from_bytes(&[0u8; MSG_MAX + 1]).is_none());
    }

    #[test]
    fn out_of_range_channel_is_none() {
        assert!(channel(MAX_CHANNELS - 1).is_some());
        assert!(channel(MAX_CHANNELS).is_none());
    }
}
