/// Length-prefixed MessagePack codec.
///
/// Wire format: `[4-byte big-endian length][MessagePack payload]`
///
/// This matches the IPC protocol spec — both Rust and Go sides
/// use the same framing so they can exchange messages over Unix
/// sockets or Windows named pipes.
use anyhow::{Context, Result, bail};
use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};

/// Maximum message size: 16 MB (prevents OOM from malformed length prefix).
const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

/// Codec for length-prefixed MessagePack messages.
pub struct Codec;

impl Codec {
    /// Encode a serializable value into a length-prefixed MessagePack frame.
    pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
        let payload = rmp_serde::to_vec(value).context("msgpack serialize")?;
        let len = payload.len() as u32;
        if len > MAX_MESSAGE_SIZE {
            bail!("message too large: {len} bytes (max {MAX_MESSAGE_SIZE})");
        }

        let mut buf = Vec::with_capacity(4 + payload.len());
        buf.put_u32(len);
        buf.extend_from_slice(&payload);
        Ok(buf)
    }

    /// Attempt to decode one frame from the buffer.
    ///
    /// Returns `Ok(Some(value))` if a complete frame was decoded (consumed bytes are removed).
    /// Returns `Ok(None)` if not enough data yet.
    /// Returns `Err` on deserialization or size errors.
    pub fn decode<T: for<'de> Deserialize<'de>>(buf: &mut BytesMut) -> Result<Option<T>> {
        if buf.len() < 4 {
            return Ok(None);
        }

        // Peek at length without consuming.
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if len > MAX_MESSAGE_SIZE {
            bail!("frame too large: {len} bytes (max {MAX_MESSAGE_SIZE})");
        }

        let total = 4 + len as usize;
        if buf.len() < total {
            return Ok(None); // Need more data.
        }

        // Consume the frame.
        buf.advance(4);
        let payload = buf.split_to(len as usize);
        let value = rmp_serde::from_slice(&payload).context("msgpack deserialize")?;
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Message, UserSpeech};

    #[test]
    fn encode_decode_roundtrip() {
        let msg = Message::UserSpeech(UserSpeech {
            text: "test".into(),
            language: "en".into(),
            confidence: 0.99,
        });

        let frame = Codec::encode(&msg).unwrap();
        assert!(frame.len() > 4);

        // First 4 bytes are the length prefix.
        let prefix_len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(prefix_len, frame.len() - 4);

        let mut buf = BytesMut::from(&frame[..]);
        let decoded: Message = Codec::decode(&mut buf).unwrap().unwrap();
        match decoded {
            Message::UserSpeech(s) => assert_eq!(s.text, "test"),
            _ => panic!("wrong variant"),
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn partial_data_returns_none() {
        let msg = Message::Ping;
        let frame = Codec::encode(&msg).unwrap();

        // Only provide half the frame.
        let mut buf = BytesMut::from(&frame[..frame.len() / 2]);
        let result: Result<Option<Message>> = Codec::decode(&mut buf);
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn multiple_messages_in_buffer() {
        let m1 = Message::Ping;
        let m2 = Message::Pong;

        let mut combined = Codec::encode(&m1).unwrap();
        combined.extend_from_slice(&Codec::encode(&m2).unwrap());

        let mut buf = BytesMut::from(&combined[..]);
        let d1: Message = Codec::decode(&mut buf).unwrap().unwrap();
        let d2: Message = Codec::decode(&mut buf).unwrap().unwrap();
        assert!(matches!(d1, Message::Ping));
        assert!(matches!(d2, Message::Pong));
        assert!(buf.is_empty());
    }
}
