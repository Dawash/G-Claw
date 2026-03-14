# G-Claw

**Portable live Jarvis** — a cross-platform native rewrite of [G](https://github.com/user/G), the voice-first AI OS.

Built with **Rust** (voice shell, brain, agents) and **Go** (tool runtime). Runs on Windows, Linux, macOS, Raspberry Pi, and Android.

## Status

**Phase 1: Voice Shell** — In Progress

| Crate | Purpose | Status |
|-------|---------|--------|
| `gclaw-ipc` | IPC protocol, MessagePack codec, TCP/Unix transport | ✅ Complete |
| `gclaw-config` | Config loader with Fernet decryption (pure Rust) | ✅ Complete |
| `gclaw-voice` | Audio capture, VAD, STT, TTS, wake word, barge-in | ✅ Core complete |

## Getting Started

### Prerequisites Checklist

Before building G-Claw, make sure you have the following installed:

- [ ] **Rust toolchain** (1.85+) -- install via [rustup](https://rustup.rs/) or run `rustup update stable`
- [ ] **Git** -- to clone the repository
- [ ] **C compiler** -- MSVC (Windows), gcc (Linux), or Xcode CLI tools (macOS)
- [ ] **LLVM/libclang** (only for `--features full`, needed by whisper-rs bindgen):
  - Windows: Download from [LLVM releases](https://releases.llvm.org/), set `LIBCLANG_PATH` env var
  - Linux: `sudo apt install libclang-dev`
  - macOS: `brew install llvm`
- [ ] **Model files** (see download commands below)
- [ ] **TTS engine** (optional):
  - [Piper](https://github.com/rhasspy/piper) binary + voice model (recommended)
  - [espeak-ng](https://github.com/espeak-ng/espeak-ng) (fallback)

### Download Models

Create the `models/` directory and download the required model files:

```bash
mkdir -p models

# Silero VAD model (voice activity detection)
curl -L -o models/silero_vad.onnx \
  https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx

# Whisper.cpp STT model (speech-to-text, ~150 MB)
curl -L -o models/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

> **Tip:** For faster inference on constrained devices (RPi, Android), use `ggml-tiny.bin` instead (~75 MB).

### Quick Start

```bash
# Verify the toolchain compiles everything (no native deps required)
cargo check

# Run the test suite
cargo test

# Build with the full voice pipeline (requires LLVM/libclang)
cargo build --release --features full
```

### Quick Demo -- IPC Round-Trip

Run the built-in IPC demo to verify that the message codec and transport work end-to-end:

```bash
cargo run --example ipc_demo -p gclaw-voice
```

This starts a local IPC server and client, sends a `UserSpeech` message, and prints the decoded round-trip result.

### First Voice Test

Once you have model files downloaded and a full build ready:

1. **Start the voice shell** (listens on port 19820 by default):
   ```bash
   cargo run --release --features full -p gclaw-voice
   ```

2. **Connect the Python brain** (from the G project root):
   ```python
   from gclaw_bridge import GclawVoiceBridge
   bridge = GclawVoiceBridge(port=19820)
   bridge.connect()
   text, lang, conf = bridge.wait_for_speech()
   print(f"You said: {text} (confidence: {conf})")
   bridge.speak("Hello from G-Claw!")
   ```

3. **Say the wake word** ("Hey G" by default), then speak your query. The voice shell transcribes your speech, sends it to the brain over IPC, and plays back the response through TTS.

## Architecture

```
gclaw-voice (Rust) ◄──IPC──► Python Brain (existing G)
                   ◄──IPC──► gclaw-tools (Go, Phase 2)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for full details.

## Build Targets

```bash
# Windows (current platform)
cargo build --release --features full

# Raspberry Pi
cross build --release --target aarch64-unknown-linux-gnu --features full

# macOS (Apple Silicon)
cargo build --release --target aarch64-apple-darwin --features full

# Linux
cargo build --release --target x86_64-unknown-linux-gnu --features full
```

## Project Structure

```
gclaw/
├── crates/
│   ├── gclaw-ipc/          # Shared IPC protocol + transport
│   ├── gclaw-config/       # Config.json reader + Fernet crypto
│   └── gclaw-voice/        # Native voice pipeline
├── python_bridge/
│   └── gclaw_bridge.py     # Drop-in speech.py replacement for Python brain
└── docs/
    ├── ARCHITECTURE.md     # System architecture overview
    ├── VOICE_PIPELINE.md   # Voice processing algorithms
    └── IPC_PROTOCOL.md     # Wire protocol specification
```

## License

MIT
