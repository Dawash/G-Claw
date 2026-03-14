/// Whisper STT via whisper-rs (whisper.cpp bindings).
///
/// Mirrors: speech.py faster-whisper/WhisperX integration.
/// Transcribes 16kHz mono f32 audio to text with language detection.
use anyhow::{Context, Result};
use tracing::{debug, info};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::Transcription;

/// Whisper model sizes and their approximate RAM usage.
#[derive(Debug, Clone, Copy)]
pub enum WhisperModel {
    /// ~75 MB RAM, fastest, lowest accuracy.
    Tiny,
    /// ~150 MB RAM, good balance for RPi.
    Base,
    /// ~500 MB RAM, good accuracy.
    Small,
    /// ~1.5 GB RAM, high accuracy (desktop default).
    Medium,
}

impl WhisperModel {
    pub fn filename(&self) -> &str {
        match self {
            Self::Tiny => "ggml-tiny.bin",
            Self::Base => "ggml-base.bin",
            Self::Small => "ggml-small.bin",
            Self::Medium => "ggml-medium.bin",
        }
    }
}

pub struct WhisperStt {
    ctx: WhisperContext,
    /// Language hint (empty = auto-detect).
    language: Option<String>,
}

impl WhisperStt {
    /// Load a Whisper model from a ggml file.
    ///
    /// `model_path`: path to ggml-{tiny,base,small,medium}.bin.
    pub fn new(model_path: &str) -> Result<Self> {
        info!("loading whisper model from {model_path}");

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| anyhow::anyhow!("load whisper model: {e}"))?;

        debug!("whisper model loaded");

        Ok(Self {
            ctx,
            language: None,
        })
    }

    /// Set language hint for STT (e.g., "en", "hi"). None = auto-detect.
    pub fn set_language(&mut self, lang: Option<String>) {
        self.language = lang;
    }

    /// Transcribe audio samples (16kHz mono f32).
    ///
    /// Returns the best transcription with language detection.
    pub fn transcribe(&self, audio: &[f32]) -> Result<Transcription> {
        if audio.is_empty() {
            return Ok(Transcription {
                text: String::new(),
                language: "en".into(),
                confidence: 0.0,
            });
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Performance settings (mirrors speech.py whisper config).
        params.set_n_threads(4);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_single_segment(true);
        params.set_no_context(true);

        // Language setting.
        if let Some(ref lang) = self.language {
            params.set_language(Some(lang));
        } else {
            params.set_language(None); // Auto-detect
        }

        // Run inference.
        let mut state = self.ctx.create_state()
            .map_err(|e| anyhow::anyhow!("create whisper state: {e}"))?;

        state.full(params, audio)
            .map_err(|e| anyhow::anyhow!("whisper transcribe: {e}"))?;

        // Collect segments.
        let num_segments = state.full_n_segments()
            .map_err(|e| anyhow::anyhow!("get segments: {e}"))?;

        let mut text = String::new();
        let mut total_prob = 0.0f32;
        let mut prob_count = 0;

        for i in 0..num_segments {
            if let Ok(segment_text) = state.full_get_segment_text(i) {
                text.push_str(&segment_text);
            }
            // Average token probabilities for confidence estimate.
            if let Ok(n_tokens) = state.full_n_tokens(i) {
                for t in 0..n_tokens {
                    if let Ok(token_data) = state.full_get_token_data(i, t) {
                        total_prob += token_data.p;
                        prob_count += 1;
                    }
                }
            }
        }

        let confidence = if prob_count > 0 {
            total_prob / prob_count as f32
        } else {
            0.0
        };

        // Detect language from model output.
        let detected_lang = state
            .full_lang_id()
            .ok()
            .and_then(|id| {
                whisper_rs::get_lang_str(id).ok().map(|s| s.to_string())
            })
            .unwrap_or_else(|| "en".into());

        let text = text.trim().to_string();
        debug!("transcribed: \"{text}\" (lang={detected_lang}, conf={confidence:.2})");

        Ok(Transcription {
            text,
            language: detected_lang,
            confidence,
        })
    }
}
