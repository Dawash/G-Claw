#[cfg(feature = "full")]
pub mod whisper;

#[cfg(feature = "full")]
pub use whisper::WhisperStt;

/// Transcription result from STT engine.
#[derive(Debug, Clone)]
pub struct Transcription {
    /// Transcribed text.
    pub text: String,
    /// Detected language code (e.g., "en", "hi", "ne").
    pub language: String,
    /// Confidence score [0.0, 1.0].
    pub confidence: f32,
}

impl Transcription {
    /// Check if this transcription is likely noise/garbage.
    ///
    /// Mirrors speech.py noise filtering:
    ///   - Too short (< 2 chars after trim)
    ///   - Common Whisper hallucinations ("Thank you.", "Thanks for watching.", etc.)
    ///   - All punctuation
    pub fn is_noise(&self) -> bool {
        let trimmed = self.text.trim();

        // Too short.
        if trimmed.len() < 2 {
            return true;
        }

        // Common Whisper hallucinations (from speech.py).
        let hallucinations = [
            "thank you.",
            "thanks for watching.",
            "thank you for watching.",
            "thanks for watching!",
            "the end.",
            "you",
            "bye.",
            "bye!",
            "bye bye.",
            "...",
            "so",
            "i'm sorry.",
        ];
        let lower = trimmed.to_lowercase();
        if hallucinations.contains(&lower.as_str()) {
            return true;
        }

        // All punctuation / whitespace.
        if trimmed.chars().all(|c| c.is_ascii_punctuation() || c.is_whitespace()) {
            return true;
        }

        // Low confidence.
        if self.confidence < 0.1 {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_detection() {
        assert!(Transcription { text: "Thank you.".into(), language: "en".into(), confidence: 0.9 }.is_noise());
        assert!(Transcription { text: "...".into(), language: "en".into(), confidence: 0.9 }.is_noise());
        assert!(Transcription { text: "a".into(), language: "en".into(), confidence: 0.9 }.is_noise());
        assert!(!Transcription { text: "hello world".into(), language: "en".into(), confidence: 0.9 }.is_noise());
    }
}
