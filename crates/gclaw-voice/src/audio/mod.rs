pub mod capture;
pub mod playback;

pub use capture::AudioCapture;
pub use playback::AudioPlayback;

/// Standard sample rate for all voice processing (VAD, STT).
pub const SAMPLE_RATE: u32 = 16_000;

/// Samples per VAD frame (512 samples @ 16kHz = 32ms).
pub const VAD_FRAME_SAMPLES: usize = 512;

/// Channels — mono for voice.
pub const CHANNELS: u16 = 1;
