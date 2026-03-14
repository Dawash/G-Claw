/// IPC transport layer — Unix domain sockets (Linux/macOS) and named pipes (Windows).
///
/// Provides async connect/listen/send/recv over the length-prefixed MessagePack codec.
use anyhow::{Context, Result};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::codec::Codec;
use crate::protocol::Message;

/// Default socket paths.
pub const VOICE_SOCKET: &str = if cfg!(windows) {
    r"\\.\pipe\gclaw-voice"
} else {
    "/tmp/gclaw-voice.sock"
};

pub const TOOLS_SOCKET: &str = if cfg!(windows) {
    r"\\.\pipe\gclaw-tools"
} else {
    "/tmp/gclaw-tools.sock"
};

/// Read buffer size for incoming data.
const READ_BUF_SIZE: usize = 8192;

/// Async IPC transport wrapping a tokio stream.
pub struct IpcTransport<S> {
    stream: S,
    read_buf: BytesMut,
}

impl<S> IpcTransport<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    /// Wrap an existing stream (e.g. from TcpStream, UnixStream, or named pipe).
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            read_buf: BytesMut::with_capacity(READ_BUF_SIZE),
        }
    }

    /// Send a message (length-prefixed MessagePack).
    pub async fn send(&mut self, msg: &Message) -> Result<()> {
        let frame = Codec::encode(msg)?;
        self.stream
            .write_all(&frame)
            .await
            .context("ipc write")?;
        self.stream.flush().await.context("ipc flush")?;
        debug!("sent {:?}", std::mem::discriminant(msg));
        Ok(())
    }

    /// Receive the next message. Returns `None` on clean EOF.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        loop {
            // Try to decode from buffered data first.
            if let Some(msg) = Codec::decode::<Message>(&mut self.read_buf)? {
                debug!("recv {:?}", std::mem::discriminant(&msg));
                return Ok(Some(msg));
            }

            // Need more data — read from stream.
            let mut tmp = [0u8; READ_BUF_SIZE];
            let n = self.stream.read(&mut tmp).await.context("ipc read")?;
            if n == 0 {
                // EOF — peer disconnected.
                if self.read_buf.is_empty() {
                    return Ok(None); // Clean disconnect.
                }
                // Partial message in buffer — this is an error, not clean EOF.
                let remaining = self.read_buf.len();
                self.read_buf.clear();
                anyhow::bail!("peer disconnected with {remaining} bytes of incomplete message");
            }
            self.read_buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Consume this transport and return the inner stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

// ---------------------------------------------------------------------------
// Platform-specific listener helpers
// ---------------------------------------------------------------------------

/// Listen on a Unix domain socket (Linux/macOS) or TCP (Windows fallback).
///
/// On Unix, removes the socket file if it already exists before binding.
#[cfg(unix)]
pub async fn listen_unix(path: &str) -> Result<tokio::net::UnixListener> {
    // Remove stale socket file.
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path).context("bind unix socket")?;
    debug!("listening on unix:{path}");
    Ok(listener)
}

/// Connect to a Unix domain socket.
#[cfg(unix)]
pub async fn connect_unix(path: &str) -> Result<IpcTransport<tokio::net::UnixStream>> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .with_context(|| format!("connect to {path}"))?;
    debug!("connected to unix:{path}");
    Ok(IpcTransport::new(stream))
}

/// On Windows, we use TCP on localhost as a portable fallback.
/// Named pipes via tokio require the `tokio::net::windows::named_pipe` module.
#[cfg(windows)]
pub async fn listen_tcp(port: u16) -> Result<tokio::net::TcpListener> {
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind tcp {addr}"))?;
    debug!("listening on tcp:{addr}");
    Ok(listener)
}

#[cfg(windows)]
pub async fn connect_tcp(port: u16) -> Result<IpcTransport<tokio::net::TcpStream>> {
    let addr = format!("127.0.0.1:{port}");
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .with_context(|| format!("connect to tcp {addr}"))?;
    debug!("connected to tcp:{addr}");
    Ok(IpcTransport::new(stream))
}

/// Default voice shell port for Windows TCP transport.
pub const VOICE_TCP_PORT: u16 = 19820;

/// Default tool runtime port for Windows TCP transport.
pub const TOOLS_TCP_PORT: u16 = 19821;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SpeakRequest, UserSpeech};

    #[tokio::test]
    async fn tcp_roundtrip() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = IpcTransport::new(stream);
            let msg = transport.recv().await.unwrap().unwrap();
            transport.send(&msg).await.unwrap(); // echo back
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut client = IpcTransport::new(stream);

        let sent = Message::UserSpeech(UserSpeech {
            text: "hello".into(),
            language: "en".into(),
            confidence: 0.9,
        });
        client.send(&sent).await.unwrap();

        let received = client.recv().await.unwrap().unwrap();
        match received {
            Message::UserSpeech(s) => assert_eq!(s.text, "hello"),
            _ => panic!("wrong message type"),
        }

        server.await.unwrap();
    }

    #[tokio::test]
    async fn send_multiple_messages() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = IpcTransport::new(stream);
            // Read 3 messages
            for _ in 0..3 {
                let msg = transport.recv().await.unwrap().unwrap();
                transport.send(&msg).await.unwrap();
            }
        });

        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut client = IpcTransport::new(stream);

        for text in ["one", "two", "three"] {
            let msg = Message::Speak(SpeakRequest { text: text.into() });
            client.send(&msg).await.unwrap();
            let echo = client.recv().await.unwrap().unwrap();
            match echo {
                Message::Speak(s) => assert_eq!(s.text, text),
                _ => panic!("wrong type"),
            }
        }

        server.await.unwrap();
    }
}
