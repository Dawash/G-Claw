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

## Quick Start

```bash
# Check (no native deps required)
cargo check

# Run tests
cargo test

# Build with full pipeline (requires LLVM/libclang for whisper-rs)
cargo build --release --features full
```

### Prerequisites for Full Build

1. **Rust** (1.85+): `rustup update stable`
2. **LLVM/libclang** (for whisper-rs bindgen):
   - Windows: Download from [LLVM releases](https://releases.llvm.org/), set `LIBCLANG_PATH`
   - Linux: `sudo apt install libclang-dev`
   - macOS: `brew install llvm`
3. **Model files** (download separately):
   - `models/silero_vad.onnx` — [Silero VAD](https://github.com/snakers4/silero-vad)
   - `models/ggml-base.bin` — [Whisper.cpp models](https://huggingface.co/ggerganov/whisper.cpp)
4. **TTS** (optional):
   - [Piper](https://github.com/rhasspy/piper) binary + voice model
   - [espeak-ng](https://github.com/espeak-ng/espeak-ng) (fallback)

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
