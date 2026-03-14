/// Voice subsystem errors.
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VoiceError {
    #[error("audio device error: {0}")]
    AudioDevice(String),

    #[error("no input device available")]
    NoInputDevice,

    #[error("no output device available")]
    NoOutputDevice,

    #[error("VAD initialization failed: {0}")]
    VadInit(String),

    #[error("VAD inference failed: {0}")]
    VadInference(String),

    #[error("STT model load failed: {0}")]
    SttModelLoad(String),

    #[error("STT transcription failed: {0}")]
    SttTranscribe(String),

    #[error("TTS synthesis failed: {0}")]
    TtsSynthesize(String),

    #[error("TTS engine not found: {0}")]
    TtsNotFound(String),

    #[error("IPC error: {0}")]
    Ipc(#[from] anyhow::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("shutdown requested")]
    Shutdown,
}
