/// Wake word detection via fuzzy string matching.
///
/// Mirrors: speech.py listen_for_wake_word() + _build_wake_words().
///
/// Flow: VAD detects speech → Whisper STT → fuzzy match against wake words.
/// Uses strsim (Jaro-Winkler) instead of Python's SequenceMatcher for speed.
use strsim::jaro_winkler;
use tracing::debug;

/// Similarity threshold for wake word matching (0.0 - 1.0).
/// Jaro-Winkler runs higher than Python's SequenceMatcher, so we use 0.78
/// (empirically tuned to match the 0.6 SequenceMatcher threshold from speech.py).
const FUZZY_THRESHOLD: f64 = 0.78;

pub struct WakeWordDetector {
    /// Set of wake word variants (lowercase).
    wake_words: Vec<String>,
    /// AI name for logging.
    ai_name: String,
}

impl WakeWordDetector {
    /// Create detector with wake word variants generated from the AI name.
    ///
    /// Mirrors speech.py _build_wake_words():
    ///   Base: "hey {name}", "{name}", "ok {name}", "yo {name}"
    ///   + Common mishearings for short names
    pub fn new(ai_name: &str) -> Self {
        let name = ai_name.to_lowercase();
        let mut words = vec![
            format!("hey {name}"),
            format!("hey {name}!"),
            name.clone(),
            format!("ok {name}"),
            format!("okay {name}"),
            format!("yo {name}"),
        ];

        // Only add generic greetings for names long enough to avoid false positives.
        if name.len() >= 3 {
            words.push(format!("hi {name}"));
            words.push(format!("hello {name}"));
        }

        // Common Whisper mishearings for short names (e.g., "G" → "gee", "ji", "jee").
        if name.len() <= 2 {
            let variants = match name.as_str() {
                "g" => vec!["gee", "ji", "jee", "gee gee", "hey gee", "hey ji"],
                "j" => vec!["jay", "hey jay"],
                _ => vec![],
            };
            for v in variants {
                words.push(v.to_string());
            }
        }

        debug!("wake words: {:?}", words);

        Self {
            wake_words: words,
            ai_name: ai_name.to_string(),
        }
    }

    /// Add a custom wake word variant.
    pub fn add_variant(&mut self, word: &str) {
        self.wake_words.push(word.to_lowercase());
    }

    /// Check if the transcribed text contains a wake word.
    ///
    /// Returns `true` if any wake word variant matches with similarity >= threshold.
    /// Checks both exact substring match and fuzzy whole-string match.
    pub fn matches(&self, text: &str) -> bool {
        let lower = text.to_lowercase().trim().to_string();

        if lower.is_empty() {
            return false;
        }

        for wake in &self.wake_words {
            // Exact substring match.
            if lower.contains(wake.as_str()) {
                debug!("wake word exact match: \"{lower}\" contains \"{wake}\"");
                return true;
            }

            // Fuzzy match on whole transcription.
            let sim = jaro_winkler(&lower, wake);
            if sim >= FUZZY_THRESHOLD {
                debug!("wake word fuzzy match: \"{lower}\" ~ \"{wake}\" (sim={sim:.2})");
                return true;
            }
        }

        // For short AI names, check individual words with a strict threshold.
        // Single-char names need near-exact match to avoid false positives.
        if self.ai_name.len() <= 3 {
            let name_lower = self.ai_name.to_lowercase();
            let strict_threshold = if self.ai_name.len() == 1 { 0.95 } else { 0.8 };
            for word in lower.split_whitespace() {
                let sim = jaro_winkler(word, &name_lower);
                if sim >= strict_threshold {
                    debug!("wake word word-level match: \"{word}\" ~ \"{name_lower}\" (sim={sim:.2})");
                    return true;
                }
            }
        }

        false
    }

    pub fn ai_name(&self) -> &str {
        &self.ai_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let d = WakeWordDetector::new("G");
        assert!(d.matches("hey G"));
        assert!(d.matches("Hey G!"));
        assert!(d.matches("ok g"));
    }

    #[test]
    fn fuzzy_match_mishearings() {
        let d = WakeWordDetector::new("G");
        assert!(d.matches("hey gee"));
        assert!(d.matches("Hey Ji"));
    }

    #[test]
    fn no_match() {
        let d = WakeWordDetector::new("G");
        assert!(!d.matches("hello world"));
        assert!(!d.matches("the weather is nice"));
        assert!(!d.matches(""));
    }

    #[test]
    fn longer_name() {
        let d = WakeWordDetector::new("Jarvis");
        assert!(d.matches("hey jarvis"));
        assert!(d.matches("Hey Jarvis, what time is it?"));
        assert!(d.matches("ok jarvis"));
    }
}
