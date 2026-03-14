pub mod piper;
pub mod espeak;

pub use piper::PiperTts;
pub use espeak::EspeakTts;

/// TTS output — raw audio ready for playback.
#[derive(Debug, Clone)]
pub struct TtsAudio {
    /// Raw f32 PCM samples.
    pub samples: Vec<f32>,
    /// Sample rate (typically 22050 for Piper).
    pub sample_rate: u32,
}

/// TTS engine trait — synthesize text to audio.
pub trait TtsEngine: Send + Sync {
    /// Synthesize text to raw audio.
    fn synthesize(&self, text: &str) -> anyhow::Result<TtsAudio>;

    /// Engine name for logging.
    fn name(&self) -> &str;

    /// Supported languages.
    fn supported_languages(&self) -> &[&str];
}

/// Select the best TTS engine for a given language.
///
/// Mirrors speech.py TTS routing:
///   English → Piper (offline, fast)
///   Other   → espeak-ng (offline fallback, multilingual)
pub fn select_engine<'a>(
    language: &str,
    piper: Option<&'a dyn TtsEngine>,
    espeak: Option<&'a dyn TtsEngine>,
) -> Option<&'a dyn TtsEngine> {
    match language {
        "en" | "english" | "" => piper.or(espeak),
        _ => espeak.or(piper),
    }
}
