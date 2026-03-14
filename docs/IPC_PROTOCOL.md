# IPC Protocol Specification

## Overview

G-Claw components communicate via IPC using length-prefixed MessagePack messages over TCP sockets (Windows) or Unix domain sockets (Linux/macOS).

## Transport

| Platform | Voice Shell | Tool Runtime |
|----------|-------------|--------------|
| Windows | TCP `127.0.0.1:19820` | TCP `127.0.0.1:19821` |
| Linux/macOS | Unix `/tmp/gclaw-voice.sock` | Unix `/tmp/gclaw-tools.sock` |

## Wire Format

```
┌──────────────────┬────────────────────────────┐
│ Length (4 bytes)  │ MessagePack Payload        │
│ big-endian u32   │ (variable length)          │
└──────────────────┴────────────────────────────┘
```

- **Maximum message size**: 16 MB (enforced by both sides)
- **Byte order**: Big-endian for length prefix
- **Encoding**: MessagePack (compact binary, faster than JSON)

## Message Envelope

Messages use MessagePack's externally tagged enum format:

**Unit variants** (no payload):
```msgpack
"WakeWordDetected"
"StopSpeaking"
"Ping"
"Pong"
```

**Tuple variants** (with payload):
```msgpack
{"UserSpeech": {"text": "hello", "language": "en", "confidence": 0.95}}
{"Speak": {"text": "Hi there!"}}
```

## Message Types

### Voice Shell → Brain

| Type | Payload | Description |
|------|---------|-------------|
| `UserSpeech` | `{text: str, language: str, confidence: f32}` | Transcribed user speech |
| `WakeWordDetected` | (none) | Wake word heard, transitioning to ACTIVE |
| `BargeIn` | `{text: str}` | User interrupted TTS playback |
| `VoiceCommand` | `{command: str}` | Meta-commands: "skip", "shorter", "repeat" |
| `Ready` | (none) | Voice shell initialized and ready |

### Brain → Voice Shell

| Type | Payload | Description |
|------|---------|-------------|
| `Speak` | `{text: str}` | Blocking TTS (no barge-in) |
| `SpeakInterruptible` | `{text: str}` | TTS with barge-in monitoring |
| `StopSpeaking` | (none) | Force-stop current TTS |
| `SetMicState` | `{state: "IDLE"\|"LISTENING"\|"PROCESSING"\|"SPEAKING"}` | Override mic state |
| `Configure` | `{stt_engine?: str, language?: str, ai_name?: str}` | Reconfigure voice shell |
| `Shutdown` | (none) | Graceful shutdown |

### Brain → Tool Runtime

| Type | Payload | Description |
|------|---------|-------------|
| `ToolExecute` | `{tool: str, args: object, user_input: str, mode: str}` | Execute a tool |

### Tool Runtime → Brain

| Type | Payload | Description |
|------|---------|-------------|
| `ToolResult` | `{result: str, success: bool, duration_ms: u64, cache_hit: bool, error?: str}` | Tool execution result |

### Bidirectional

| Type | Payload | Description |
|------|---------|-------------|
| `Ping` | (none) | Health check request |
| `Pong` | (none) | Health check response |

## Python Client Example

```python
import socket, struct, msgpack

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.connect(("127.0.0.1", 19820))

# Send a Speak message
msg = {"Speak": {"text": "Hello world"}}
payload = msgpack.packb(msg, use_bin_type=True)
sock.sendall(struct.pack(">I", len(payload)) + payload)

# Receive response
header = sock.recv(4)
length = struct.unpack(">I", header)[0]
data = sock.recv(length)
response = msgpack.unpackb(data, raw=False)
```

## Error Handling

- If a peer disconnects, the other side detects EOF and logs a warning
- Malformed messages (bad msgpack, unknown variant) are logged and skipped
- Messages larger than 16 MB are rejected with an error
- Ping/Pong can be used for health checks (timeout: 5s recommended)

## Versioning

The protocol currently has no version negotiation. All components in a deployment must use the same protocol version. Future versions may add a handshake with version exchange.
