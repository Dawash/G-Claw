/// Python IPC Bridge — adapter for assistant_loop.py to communicate with gclaw-voice.
///
/// This module provides the Rust-side server that listens for connections from
/// the Python brain. In the hybrid Phase 1-2 architecture:
///
///   Python (assistant_loop.py) ──IPC──▶ Rust (gclaw-voice)
///
/// The Python side uses a thin bridge class (see gclaw_bridge.py) that replaces
/// direct calls to speech.listen() / speech.speak() with IPC messages.
///
/// This module is used when gclaw-voice is the IPC *server* (brain connects to it).
use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use gclaw_ipc::transport::IpcTransport;

/// Start the IPC server for brain connections.
///
/// Listens on `port` and accepts one connection (the Python brain).
/// Returns the transport for use in the voice shell main loop.
pub async fn start_ipc_server(port: u16) -> Result<IpcTransport<tokio::net::TcpStream>> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    info!("voice shell IPC server listening on port {port}");

    let (stream, addr) = listener.accept().await?;
    info!("brain connected from {addr}");

    Ok(IpcTransport::new(stream))
}
