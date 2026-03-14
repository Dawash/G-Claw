/// Integration tests for IPC protocol + codec + transport.
use gclaw_ipc::codec::Codec;
use gclaw_ipc::protocol::*;
use gclaw_ipc::transport::IpcTransport;
use bytes::BytesMut;

#[test]
fn all_message_types_roundtrip_via_codec() {
    let messages: Vec<Message> = vec![
        // Unit variants
        Message::WakeWordDetected,
        Message::Ready,
        Message::StopSpeaking,
        Message::Shutdown,
        Message::Ping,
        Message::Pong,
        // Tuple variants
        Message::UserSpeech(UserSpeech {
            text: "hello".into(),
            language: "en".into(),
            confidence: 0.95,
        }),
        Message::BargeIn(BargeIn {
            text: "wait".into(),
        }),
        Message::VoiceCommand(VoiceCommand {
            command: "skip".into(),
        }),
        Message::Speak(SpeakRequest {
            text: "Hi there!".into(),
        }),
        Message::SpeakInterruptible(SpeakRequest {
            text: "Long response...".into(),
        }),
        Message::SetMicState(SetMicStateRequest {
            state: MicState::Listening,
        }),
        Message::Configure(ConfigureVoice {
            stt_engine: Some("whisper".into()),
            language: Some("en".into()),
            ai_name: Some("G".into()),
        }),
        Message::ToolExecute(ToolExecute {
            tool: "get_weather".into(),
            args: serde_json::json!({"city": "Tokyo"}),
            user_input: "weather in Tokyo".into(),
            mode: "quick".into(),
        }),
        Message::ToolResult(ToolResult {
            result: "Sunny, 25°C".into(),
            success: true,
            duration_ms: 150,
            cache_hit: false,
            error: None,
        }),
    ];

    for msg in &messages {
        let encoded = Codec::encode(msg).expect("encode");
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded: Message = Codec::decode(&mut buf)
            .expect("decode")
            .expect("should have data");

        // Verify discriminant matches (can't easily deep-compare enums).
        assert_eq!(
            std::mem::discriminant(msg),
            std::mem::discriminant(&decoded),
            "discriminant mismatch for {msg:?}"
        );
    }
}

#[test]
fn codec_handles_large_payload() {
    let big_text = "x".repeat(100_000);
    let msg = Message::Speak(SpeakRequest { text: big_text.clone() });
    let encoded = Codec::encode(&msg).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded: Message = Codec::decode(&mut buf).unwrap().unwrap();
    match decoded {
        Message::Speak(s) => assert_eq!(s.text.len(), 100_000),
        _ => panic!("wrong type"),
    }
}

#[test]
fn codec_rejects_oversized_frame() {
    // Craft a frame header claiming 20MB.
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&(20_000_000u32).to_be_bytes());
    buf.extend_from_slice(&[0u8; 100]);
    let result: Result<Option<Message>, _> = Codec::decode(&mut buf);
    assert!(result.is_err());
}

#[tokio::test]
async fn transport_handles_rapid_messages() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut transport = IpcTransport::new(stream);
        let mut count = 0;
        while let Ok(Some(_)) = transport.recv().await {
            count += 1;
            if count >= 100 {
                break;
            }
        }
        assert_eq!(count, 100);
    });

    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut client = IpcTransport::new(stream);

    for i in 0..100 {
        let msg = Message::UserSpeech(UserSpeech {
            text: format!("message {i}"),
            language: "en".into(),
            confidence: 0.9,
        });
        client.send(&msg).await.unwrap();
    }

    drop(client); // Close connection so server sees EOF.
    server.await.unwrap();
}

#[tokio::test]
async fn transport_clean_disconnect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut transport = IpcTransport::new(stream);
        // Peer closes cleanly — should return None.
        let result = transport.recv().await.unwrap();
        assert!(result.is_none());
    });

    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    drop(stream); // Immediate close.

    server.await.unwrap();
}
