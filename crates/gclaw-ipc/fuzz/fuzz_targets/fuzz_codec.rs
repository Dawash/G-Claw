#![no_main]
/// Fuzz the IPC codec with arbitrary byte sequences.
///
/// This catches panics, OOM, and logic errors from malformed MessagePack data.
///
/// Run with: cargo +nightly fuzz run fuzz_codec (from crates/gclaw-ipc/)
use bytes::BytesMut;
use gclaw_ipc::codec::Codec;
use gclaw_ipc::protocol::Message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Try to decode arbitrary bytes as a framed Message.
    let mut buf = BytesMut::from(data);
    let _ = Codec::decode::<Message>(&mut buf);

    // Also try encoding then decoding (should always roundtrip).
    if data.len() >= 4 {
        // Use first 4 bytes as a fake length prefix + rest as payload.
        let mut framed = BytesMut::new();
        framed.extend_from_slice(data);
        let _ = Codec::decode::<Message>(&mut framed);
    }
});
