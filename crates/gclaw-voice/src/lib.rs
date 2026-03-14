/// gclaw-voice: Native voice shell for G-Claw.
///
/// Replaces G/speech.py (1,662 lines) with a native Rust binary handling:
///   - Audio capture/playback (cpal)
///   - Voice Activity Detection (Silero VAD via ONNX Runtime)
///   - Speech-to-Text (whisper.cpp via whisper-rs)
///   - Text-to-Speech (Piper binary + espeak-ng fallback)
///   - Wake word detection (fuzzy string matching)
///   - Barge-in (concurrent TTS + VAD monitoring)
///
/// Communicates with the Python brain via IPC (MessagePack over TCP/Unix socket).
///
/// Feature gates:
///   `full` — enables whisper-rs (STT) and ort (VAD). Requires LLVM/libclang.
///   Without `full`, only audio, TTS, wake word, IPC, and state modules are available.

pub mod audio;
#[cfg(feature = "full")]
pub mod vad;
pub mod stt;
pub mod tts;
pub mod wake;
#[cfg(feature = "full")]
pub mod bargein;
pub mod state;
#[cfg(feature = "full")]
pub mod shell;
pub mod bridge;
pub mod error;

pub use state::MicState;
#[cfg(feature = "full")]
pub use shell::VoiceShell;
pub use error::VoiceError;
