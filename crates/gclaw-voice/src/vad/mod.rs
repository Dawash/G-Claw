pub mod silero;

pub use silero::SileroVad;

/// VAD decision for a single audio frame.
#[derive(Debug, Clone)]
pub enum VadDecision {
    /// No speech detected in this frame.
    Silence,
    /// Speech just started.
    SpeechStart,
    /// Speech is continuing.
    SpeechContinue,
    /// Speech ended — contains the full speech segment.
    SpeechEnd(SpeechSegment),
}

/// A complete speech segment extracted by VAD.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    /// Raw audio samples (16kHz mono f32).
    pub audio: Vec<f32>,
    /// Duration in seconds.
    pub duration_secs: f32,
    /// Timestamp when speech started (samples from stream start).
    pub start_sample: u64,
}

impl SpeechSegment {
    pub fn new(audio: Vec<f32>, start_sample: u64) -> Self {
        let duration_secs = audio.len() as f32 / 16_000.0;
        Self {
            audio,
            duration_secs,
            start_sample,
        }
    }
}
