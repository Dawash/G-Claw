/// IPC demo — shows the voice shell's IPC protocol in action.
///
/// Run with: cargo run --example ipc_demo
///
/// This starts a mock "brain" server and a simulated voice client,
/// demonstrating the full message flow without requiring audio hardware.
use gclaw_ipc::protocol::*;
use gclaw_ipc::transport::IpcTransport;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== G-Claw IPC Demo ===\n");

    // Start a mock brain server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    println!("Mock brain listening on {addr}");

    // Spawn mock brain.
    let brain = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut transport = IpcTransport::new(stream);

        // Wait for Ready.
        let msg = transport.recv().await.unwrap().unwrap();
        println!("[brain] received: {msg:?}");

        // Wait for wake word.
        let msg = transport.recv().await.unwrap().unwrap();
        println!("[brain] received: {msg:?}");

        // Wait for user speech.
        let msg = transport.recv().await.unwrap().unwrap();
        println!("[brain] received: {msg:?}");

        // Send a response.
        let response = Message::Speak(SpeakRequest {
            text: "The weather in London is 15 degrees and cloudy.".into(),
        });
        println!("[brain] sending: {response:?}");
        transport.send(&response).await.unwrap();

        // Ping/pong health check.
        transport.send(&Message::Ping).await.unwrap();
        let pong = transport.recv().await.unwrap().unwrap();
        println!("[brain] health check: {pong:?}");

        // Shutdown.
        transport.send(&Message::Shutdown).await.unwrap();
        println!("[brain] sent shutdown");
    });

    // Small delay to let server start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Simulate voice shell client.
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let mut voice = IpcTransport::new(stream);

    // Send Ready.
    println!("[voice] sending Ready");
    voice.send(&Message::Ready).await?;

    // Simulate wake word detection.
    println!("[voice] wake word detected!");
    voice.send(&Message::WakeWordDetected).await?;

    // Simulate user speech.
    let speech = Message::UserSpeech(UserSpeech {
        text: "What's the weather in London?".into(),
        language: "en".into(),
        confidence: 0.94,
    });
    println!("[voice] sending: {speech:?}");
    voice.send(&speech).await?;

    // Receive brain's response.
    let response = voice.recv().await?.unwrap();
    match &response {
        Message::Speak(req) => println!("[voice] brain says: \"{}\"", req.text),
        other => println!("[voice] unexpected: {other:?}"),
    }

    // Handle ping.
    let ping = voice.recv().await?.unwrap();
    if matches!(ping, Message::Ping) {
        voice.send(&Message::Pong).await?;
    }

    // Handle shutdown.
    let shutdown = voice.recv().await?.unwrap();
    println!("[voice] received: {shutdown:?}");

    brain.await?;

    println!("\n=== Demo complete! ===");
    println!("Full message flow: Ready → WakeWord → UserSpeech → Speak → Ping/Pong → Shutdown");
    Ok(())
}
