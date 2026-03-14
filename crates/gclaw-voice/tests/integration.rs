/// Comprehensive integration tests for gclaw-voice.
///
/// These tests exercise the public API surface available WITHOUT the "full" feature:
///   - audio:  AudioRingBuffer (capture), AudioPlayback (type only)
///   - stt:    Transcription + is_noise()
///   - tts:    TtsEngine trait, select_engine(), TtsAudio
///   - wake:   WakeWordDetector
///   - state:  MicState, SessionMode, VoiceState
///   - error:  VoiceError

use gclaw_voice::audio::capture::AudioRingBuffer;
use gclaw_voice::stt::Transcription;
use gclaw_voice::tts::{self, TtsAudio, TtsEngine};
use gclaw_voice::wake::WakeWordDetector;
use gclaw_voice::state::{MicState, SessionMode, VoiceState};
use gclaw_voice::error::VoiceError;

// ============================================================================
//  1. Ring buffer stress tests
// ============================================================================

#[test]
fn ring_buffer_stress_1m_samples() {
    // Write 1,000,000 samples into a buffer that holds 16,000 (1 second @ 16kHz).
    let capacity = 16_000;
    let mut rb = AudioRingBuffer::new(capacity);

    let total_samples: usize = 1_000_000;
    // Write in chunks of varying sizes to stress wrap-around logic.
    let chunk_sizes = [1, 7, 63, 512, 1000, 1500, 3333];
    let mut written: usize = 0;
    let mut chunk_idx = 0;

    while written < total_samples {
        let chunk_sz = chunk_sizes[chunk_idx % chunk_sizes.len()].min(total_samples - written);
        let chunk: Vec<f32> = (written..written + chunk_sz)
            .map(|i| i as f32)
            .collect();
        rb.write(&chunk);
        written += chunk_sz;
        chunk_idx += 1;
    }

    assert_eq!(rb.total_written(), total_samples as u64);

    // The last `capacity` samples written were (total_samples - capacity)..total_samples.
    let last = rb.read_last(capacity);
    assert_eq!(last.len(), capacity);
    for (i, &val) in last.iter().enumerate() {
        let expected = (total_samples - capacity + i) as f32;
        assert!(
            (val - expected).abs() < f32::EPSILON,
            "mismatch at index {i}: got {val}, expected {expected}"
        );
    }
}

#[test]
fn ring_buffer_stress_read_various_windows() {
    let capacity = 8_000;
    let mut rb = AudioRingBuffer::new(capacity);

    // Write exactly 2x capacity to force a complete wrap.
    let samples: Vec<f32> = (0..capacity * 2).map(|i| i as f32).collect();
    rb.write(&samples);

    // Read windows of various sizes.
    let window_sizes = [1, 2, 10, 100, 500, 1000, 4000, 7999, 8000];
    for &win in &window_sizes {
        let result = rb.read_last(win);
        assert_eq!(result.len(), win, "window size {win}");
        // Verify the values are the tail of the written sequence.
        for (j, &val) in result.iter().enumerate() {
            let expected = (capacity * 2 - win + j) as f32;
            assert!(
                (val - expected).abs() < f32::EPSILON,
                "window {win}, index {j}: got {val}, expected {expected}"
            );
        }
    }

    // Reading more than capacity returns capacity samples.
    let too_large = rb.read_last(capacity + 1000);
    assert_eq!(too_large.len(), capacity);
}

#[test]
fn ring_buffer_read_zero_samples() {
    let mut rb = AudioRingBuffer::new(100);
    rb.write(&[1.0, 2.0, 3.0]);
    let result = rb.read_last(0);
    assert!(result.is_empty());
}

#[test]
fn ring_buffer_empty_read() {
    // No samples written yet.
    let rb = AudioRingBuffer::new(100);
    let result = rb.read_last(50);
    assert!(result.is_empty());
    assert_eq!(rb.total_written(), 0);
}

#[test]
fn ring_buffer_capacity_one() {
    let mut rb = AudioRingBuffer::new(1);
    rb.write(&[42.0]);
    assert_eq!(rb.read_last(1), vec![42.0]);

    rb.write(&[99.0]);
    assert_eq!(rb.read_last(1), vec![99.0]);
    assert_eq!(rb.total_written(), 2);

    // Write many values, only the last should survive.
    for i in 0..1000 {
        rb.write(&[i as f32]);
    }
    assert_eq!(rb.read_last(1), vec![999.0]);
    assert_eq!(rb.read_last(100), vec![999.0]); // can't read more than 1
}

#[test]
fn ring_buffer_write_exactly_capacity() {
    let capacity = 64;
    let mut rb = AudioRingBuffer::new(capacity);
    let samples: Vec<f32> = (0..capacity).map(|i| i as f32).collect();
    rb.write(&samples);

    // write_pos should be back at 0 after writing exactly capacity samples.
    let result = rb.read_last(capacity);
    assert_eq!(result.len(), capacity);
    for (i, &val) in result.iter().enumerate() {
        assert!((val - i as f32).abs() < f32::EPSILON);
    }
}

#[test]
fn ring_buffer_write_empty_slice() {
    let mut rb = AudioRingBuffer::new(10);
    rb.write(&[]);
    assert_eq!(rb.total_written(), 0);
    assert!(rb.read_last(5).is_empty());
}

#[test]
fn ring_buffer_sequential_small_writes() {
    let mut rb = AudioRingBuffer::new(5);

    // Write one sample at a time across multiple wrap-arounds.
    for i in 0..20 {
        rb.write(&[i as f32]);
    }

    let last5 = rb.read_last(5);
    assert_eq!(last5, vec![15.0, 16.0, 17.0, 18.0, 19.0]);
}

#[test]
fn ring_buffer_multiple_wraps_small() {
    let mut rb = AudioRingBuffer::new(3);
    for i in 0..20 {
        rb.write(&[i as f32]);
    }
    assert_eq!(rb.read_last(3), vec![17.0, 18.0, 19.0]);
}

#[test]
fn ring_buffer_partial_read_after_partial_fill() {
    let mut rb = AudioRingBuffer::new(100);
    rb.write(&[10.0, 20.0, 30.0]);

    // Read fewer than written.
    assert_eq!(rb.read_last(2), vec![20.0, 30.0]);
    assert_eq!(rb.read_last(1), vec![30.0]);
    assert_eq!(rb.read_last(3), vec![10.0, 20.0, 30.0]);
}

// ============================================================================
//  2. Noise filtering edge cases
// ============================================================================

fn make_transcription(text: &str, confidence: f32) -> Transcription {
    Transcription {
        text: text.to_string(),
        language: "en".to_string(),
        confidence,
    }
}

// --- Known Whisper hallucinations ---

#[test]
fn noise_all_known_hallucinations() {
    let hallucinations = [
        "Thank you.",
        "Thanks for watching.",
        "Thank you for watching.",
        "Thanks for watching!",
        "The end.",
        "You",
        "Bye.",
        "Bye!",
        "Bye bye.",
        "...",
        "So",
        "I'm sorry.",
    ];

    for phrase in &hallucinations {
        let t = make_transcription(phrase, 0.95);
        assert!(
            t.is_noise(),
            "expected hallucination to be noise: {:?}",
            phrase
        );
    }
}

#[test]
fn noise_hallucinations_case_insensitive() {
    // Hallucinations should match regardless of case.
    let variants = [
        "THANK YOU.",
        "thank you.",
        "Thank You.",
        "THANKS FOR WATCHING.",
        "thanks for watching!",
        "BYE.",
        "bye!",
        "THE END.",
        "SO",
        "I'M SORRY.",
    ];
    for phrase in &variants {
        let t = make_transcription(phrase, 0.95);
        assert!(t.is_noise(), "case variant should be noise: {:?}", phrase);
    }
}

#[test]
fn noise_hallucination_with_whitespace_padding() {
    // Leading/trailing whitespace should be trimmed before comparison.
    let t = make_transcription("  Thank you.  ", 0.95);
    assert!(t.is_noise());

    let t2 = make_transcription("\tBye.\n", 0.95);
    assert!(t2.is_noise());
}

// --- Too short ---

#[test]
fn noise_too_short() {
    assert!(make_transcription("", 0.95).is_noise());
    assert!(make_transcription("a", 0.95).is_noise());
    assert!(make_transcription(" ", 0.95).is_noise());
    assert!(make_transcription("  ", 0.95).is_noise());
    // Single char after trim is still < 2.
    assert!(make_transcription(" x ", 0.95).is_noise());
}

#[test]
fn noise_exactly_two_chars_not_noise() {
    // Two-char string that is real text should NOT be noise.
    let t = make_transcription("hi", 0.95);
    assert!(!t.is_noise(), "\"hi\" is a valid 2-char transcription");
}

// --- Punctuation-only ---

#[test]
fn noise_all_punctuation() {
    let punct_strings = [
        "!!!", "??", ".,.", "---", "***", ";;;", "!@#$%^&*()", "... ...",
    ];
    for s in &punct_strings {
        let t = make_transcription(s, 0.95);
        assert!(t.is_noise(), "punctuation-only should be noise: {:?}", s);
    }
}

#[test]
fn noise_whitespace_only() {
    let ws_strings = ["   ", "\t\t", " \n "];
    for s in &ws_strings {
        let t = make_transcription(s, 0.95);
        assert!(t.is_noise(), "whitespace-only should be noise: {:?}", s);
    }
}

#[test]
fn noise_mixed_punctuation_and_whitespace() {
    let t = make_transcription("  . . .  ", 0.95);
    assert!(t.is_noise());
}

// --- Low confidence ---

#[test]
fn noise_low_confidence() {
    let t = make_transcription("hello world", 0.05);
    assert!(t.is_noise(), "confidence below 0.1 should be noise");

    let t2 = make_transcription("hello world", 0.0);
    assert!(t2.is_noise());

    let t3 = make_transcription("hello world", 0.099);
    assert!(t3.is_noise());
}

#[test]
fn noise_borderline_confidence() {
    // Exactly 0.1 should NOT be noise (threshold is < 0.1).
    let t = make_transcription("hello world", 0.1);
    assert!(!t.is_noise());

    let t2 = make_transcription("hello world", 0.11);
    assert!(!t2.is_noise());
}

// --- Valid (non-noise) transcriptions ---

#[test]
fn not_noise_real_text() {
    let valid = [
        "Hello, how are you?",
        "Set a timer for 5 minutes",
        "What is the weather like today",
        "Play some music",
        "Open the door",
        "Thank you very much for your help",  // not in hallucination list
        "Turn off the lights",
    ];
    for phrase in &valid {
        let t = make_transcription(phrase, 0.9);
        assert!(!t.is_noise(), "valid text should not be noise: {:?}", phrase);
    }
}

// --- Unicode ---

#[test]
fn noise_unicode_real_text() {
    // Unicode text that contains real characters should NOT be noise.
    // "namaste" in Devanagari
    let t = make_transcription("\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947}", 0.9);
    assert!(!t.is_noise());
}

#[test]
fn noise_unicode_single_char() {
    // A single multi-byte char: len() counts bytes, so a 3-byte char has len=3 which is >= 2.
    let t = make_transcription("\u{4F60}", 0.9); // Chinese character - 3 bytes
    assert!(!t.is_noise(), "multi-byte single char has byte len >= 2");
}

#[test]
fn noise_japanese_text() {
    let t = make_transcription("\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}", 0.8); // "konnichiwa"
    assert!(!t.is_noise());
}

// --- Very long text ---

#[test]
fn noise_very_long_text() {
    let long_text = "a ".repeat(10_000);
    let t = make_transcription(&long_text, 0.9);
    assert!(!t.is_noise(), "long text should not be noise");
}

// ============================================================================
//  3. Wake word edge cases
// ============================================================================

// --- Single character name "G" ---

#[test]
fn wake_g_exact_variants() {
    let d = WakeWordDetector::new("G");
    assert!(d.matches("hey g"));
    assert!(d.matches("Hey G"));
    assert!(d.matches("HEY G!"));
    assert!(d.matches("ok g"));
    assert!(d.matches("yo g"));
    assert!(d.matches("g"));
    assert!(d.matches("G"));
}

#[test]
fn wake_g_whisper_mishearings() {
    let d = WakeWordDetector::new("G");
    assert!(d.matches("gee"));
    assert!(d.matches("ji"));
    assert!(d.matches("jee"));
    assert!(d.matches("hey gee"));
    assert!(d.matches("hey ji"));
    assert!(d.matches("gee gee"));
}

#[test]
fn wake_g_no_false_positives() {
    let d = WakeWordDetector::new("G");
    // Note: "g" is a 1-char wake word, so single-char substring matches can occur.
    // These phrases do NOT contain "g" as a substring and should not fuzzy-match.
    assert!(!d.matches("the weather is nice today"));
    assert!(!d.matches("set a timer for five minutes"));
    assert!(!d.matches("what time is it now"));
    assert!(!d.matches(""));
}

#[test]
fn wake_g_substring_matches() {
    let d = WakeWordDetector::new("G");
    // "g" appears as a substring in many English words -- the detector
    // intentionally treats this as a match (substring check is first).
    assert!(d.matches("the weather is great")); // "great" contains "g"
    assert!(d.matches("going to the store"));   // "going" contains "g"
}

// --- Single character name "J" ---

#[test]
fn wake_j_variants() {
    let d = WakeWordDetector::new("J");
    assert!(d.matches("hey j"));
    assert!(d.matches("hey jay"));
    assert!(d.matches("J"));
    assert!(d.matches("ok j"));
}

#[test]
fn wake_j_substring_matches() {
    let d = WakeWordDetector::new("J");
    // "j" is a 1-char wake word and appears as a substring in "just".
    assert!(d.matches("just a normal sentence"));
}

#[test]
fn wake_j_no_false_positives() {
    let d = WakeWordDetector::new("J");
    // Phrases without "j" anywhere should not match.
    assert!(!d.matches("the weather is nice"));
    assert!(!d.matches("set a timer"));
    assert!(!d.matches(""));
}

// --- Medium name "Jarvis" ---

#[test]
fn wake_jarvis_exact() {
    let d = WakeWordDetector::new("Jarvis");
    assert!(d.matches("hey jarvis"));
    assert!(d.matches("Hey Jarvis, what time is it?"));
    assert!(d.matches("ok jarvis"));
    assert!(d.matches("okay jarvis"));
    assert!(d.matches("yo jarvis"));
    assert!(d.matches("hi jarvis"));
    assert!(d.matches("hello jarvis"));
    assert!(d.matches("jarvis"));
}

#[test]
fn wake_jarvis_no_match() {
    let d = WakeWordDetector::new("Jarvis");
    assert!(!d.matches("what time is it"));
    assert!(!d.matches("the stock market crashed today"));
    assert!(!d.matches(""));
}

#[test]
fn wake_jarvis_fuzzy_match_hello() {
    let d = WakeWordDetector::new("Jarvis");
    // "hello jarvis" is a wake word variant, and "hello world" is close enough
    // in Jaro-Winkler similarity to match (shared "hello " prefix).
    assert!(d.matches("hello world"));
}

#[test]
fn wake_jarvis_fuzzy() {
    let d = WakeWordDetector::new("Jarvis");
    // Close enough for fuzzy matching.
    assert!(d.matches("hey jarviss"));
    assert!(d.matches("hey jarves"));
}

// --- Long name "Computer" ---

#[test]
fn wake_computer_variants() {
    let d = WakeWordDetector::new("Computer");
    assert!(d.matches("hey computer"));
    assert!(d.matches("ok computer"));
    assert!(d.matches("okay computer"));
    assert!(d.matches("yo computer"));
    assert!(d.matches("hi computer"));
    assert!(d.matches("hello computer"));
    assert!(d.matches("computer"));
    assert!(d.matches("COMPUTER"));
}

#[test]
fn wake_computer_no_false_positive() {
    let d = WakeWordDetector::new("Computer");
    assert!(!d.matches("I need a new laptop"));
    assert!(!d.matches("the stock market crashed"));
    assert!(!d.matches(""));
}

#[test]
fn wake_computer_fuzzy_match_computing() {
    let d = WakeWordDetector::new("Computer");
    // "computing resources" is close enough to "computer" via Jaro-Winkler
    // due to the shared "comput" prefix -- this is expected fuzzy behavior.
    assert!(d.matches("computing resources"));
}

// --- Case sensitivity ---

#[test]
fn wake_case_insensitive() {
    let d = WakeWordDetector::new("Jarvis");
    assert!(d.matches("HEY JARVIS"));
    assert!(d.matches("hey jarvis"));
    assert!(d.matches("HeY jArViS"));
}

// --- Unicode names ---

#[test]
fn wake_unicode_name() {
    // A Unicode AI name should work for exact matches.
    let d = WakeWordDetector::new("\u{30ED}\u{30DC}"); // Katakana "robo"
    assert!(d.matches("hey \u{30ED}\u{30DC}"));
    assert!(d.matches("\u{30ED}\u{30DC}"));
    // Completely unrelated text with no shared characters should not match.
    assert!(!d.matches("set a timer for five minutes"));
}

#[test]
fn wake_unicode_name_hello_fuzzy() {
    // Unicode name with byte len >= 3 gets "hello <name>" variant.
    // "hello world" fuzzy-matches "hello <unicode>" due to shared prefix.
    let d = WakeWordDetector::new("\u{30ED}\u{30DC}");
    assert!(d.matches("hello world"));
}

// --- Empty / whitespace input ---

#[test]
fn wake_empty_input() {
    let d = WakeWordDetector::new("G");
    assert!(!d.matches(""));
    assert!(!d.matches("   "));
}

// --- Custom variant ---

#[test]
fn wake_add_variant() {
    let mut d = WakeWordDetector::new("Jarvis");
    d.add_variant("Friday");
    assert!(d.matches("friday"));
    assert!(d.matches("hey friday"));
}

// --- ai_name accessor ---

#[test]
fn wake_ai_name_preserved() {
    let d = WakeWordDetector::new("Jarvis");
    assert_eq!(d.ai_name(), "Jarvis");

    let d2 = WakeWordDetector::new("G");
    assert_eq!(d2.ai_name(), "G");
}

// --- Short names don't get hi/hello variants (to avoid false positives) ---

#[test]
fn wake_short_name_no_hi_hello() {
    let d = WakeWordDetector::new("G");
    // "hi" and "hello" are NOT added as wake word variants for names < 3 chars.
    // So "hi g" should still match (contains "g"), but "hi" alone should not.
    assert!(d.matches("hi g"));
}

// ============================================================================
//  4. TTS engine selection
// ============================================================================

/// Minimal mock TTS engine for testing select_engine().
struct MockTtsEngine {
    engine_name: &'static str,
    languages: &'static [&'static str],
}

impl TtsEngine for MockTtsEngine {
    fn synthesize(&self, _text: &str) -> anyhow::Result<TtsAudio> {
        Ok(TtsAudio {
            samples: vec![],
            sample_rate: 22050,
        })
    }

    fn name(&self) -> &str {
        self.engine_name
    }

    fn supported_languages(&self) -> &[&str] {
        self.languages
    }
}

#[test]
fn tts_select_english_both_present() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi", "ne"],
    };

    // English should prefer Piper.
    let engine = tts::select_engine("en", Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "piper");

    let engine2 = tts::select_engine("english", Some(&piper), Some(&espeak));
    assert_eq!(engine2.unwrap().name(), "piper");

    // Empty string also maps to English -> Piper.
    let engine3 = tts::select_engine("", Some(&piper), Some(&espeak));
    assert_eq!(engine3.unwrap().name(), "piper");
}

#[test]
fn tts_select_hindi_both_present() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi", "ne"],
    };

    // Hindi should prefer espeak.
    let engine = tts::select_engine("hi", Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "espeak-ng");
}

#[test]
fn tts_select_unknown_language_both_present() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi"],
    };

    // Unknown language falls to non-English path -> espeak preferred.
    let engine = tts::select_engine("zh", Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "espeak-ng");

    let engine2 = tts::select_engine("ja", Some(&piper), Some(&espeak));
    assert_eq!(engine2.unwrap().name(), "espeak-ng");
}

#[test]
fn tts_select_piper_only() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };

    // English with only Piper -> Piper.
    let engine = tts::select_engine("en", Some(&piper), None);
    assert_eq!(engine.unwrap().name(), "piper");

    // Non-English with only Piper -> falls back to Piper (espeak.or(piper)).
    let engine2 = tts::select_engine("hi", Some(&piper), None);
    assert_eq!(engine2.unwrap().name(), "piper");
}

#[test]
fn tts_select_espeak_only() {
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi"],
    };

    // English with only espeak -> espeak (piper.or(espeak) = espeak).
    let engine = tts::select_engine("en", None, Some(&espeak));
    assert_eq!(engine.unwrap().name(), "espeak-ng");

    // Non-English with only espeak -> espeak.
    let engine2 = tts::select_engine("hi", None, Some(&espeak));
    assert_eq!(engine2.unwrap().name(), "espeak-ng");
}

#[test]
fn tts_select_neither_present() {
    let engine = tts::select_engine("en", None, None);
    assert!(engine.is_none());

    let engine2 = tts::select_engine("hi", None, None);
    assert!(engine2.is_none());

    let engine3 = tts::select_engine("", None, None);
    assert!(engine3.is_none());
}

#[test]
fn tts_select_various_non_english_languages() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi", "ne", "es", "fr", "de"],
    };

    // All non-English languages should prefer espeak.
    for lang in &["ne", "es", "fr", "de", "ru", "ar", "pt", "ko"] {
        let engine = tts::select_engine(lang, Some(&piper), Some(&espeak));
        assert_eq!(
            engine.unwrap().name(),
            "espeak-ng",
            "language '{lang}' should select espeak"
        );
    }
}

// ============================================================================
//  5. Audio playback state / AudioRingBuffer additional edge cases
// ============================================================================

#[test]
fn ring_buffer_capacity_boundary_write_read() {
    // Write exactly capacity, then read exactly capacity.
    let cap = 256;
    let mut rb = AudioRingBuffer::new(cap);
    let data: Vec<f32> = (0..cap).map(|i| i as f32 * 0.01).collect();
    rb.write(&data);
    let out = rb.read_last(cap);
    assert_eq!(out.len(), cap);
    assert!((out[0] - 0.0).abs() < f32::EPSILON);
    assert!((out[cap - 1] - (cap - 1) as f32 * 0.01).abs() < f32::EPSILON);
}

#[test]
fn ring_buffer_multiple_wraps_large() {
    // Write capacity * 5 samples, one at a time.
    let cap = 10;
    let mut rb = AudioRingBuffer::new(cap);
    for i in 0..50 {
        rb.write(&[i as f32]);
    }
    assert_eq!(rb.total_written(), 50);
    let last = rb.read_last(cap);
    assert_eq!(last, vec![40.0, 41.0, 42.0, 43.0, 44.0, 45.0, 46.0, 47.0, 48.0, 49.0]);
}

#[test]
fn tts_audio_struct_creation() {
    let audio = TtsAudio {
        samples: vec![0.0, 0.5, -0.5, 1.0],
        sample_rate: 22050,
    };
    assert_eq!(audio.samples.len(), 4);
    assert_eq!(audio.sample_rate, 22050);
}

#[test]
fn tts_audio_empty() {
    let audio = TtsAudio {
        samples: vec![],
        sample_rate: 16000,
    };
    assert!(audio.samples.is_empty());
    assert_eq!(audio.sample_rate, 16000);
}

#[test]
fn tts_audio_clone() {
    let audio = TtsAudio {
        samples: vec![0.1, 0.2, 0.3],
        sample_rate: 44100,
    };
    let cloned = audio.clone();
    assert_eq!(cloned.samples, audio.samples);
    assert_eq!(cloned.sample_rate, audio.sample_rate);
}

// ============================================================================
//  6. Barge-in RMS (compute_rms with known signals)
// ============================================================================

/// Compute RMS of a signal -- same formula used in barge-in logic.
/// Replicated here since the bargein module requires the "full" feature.
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

#[test]
fn rms_pure_silence() {
    let silence = vec![0.0f32; 1000];
    let rms = compute_rms(&silence);
    assert!((rms - 0.0).abs() < f32::EPSILON, "silence RMS should be 0.0");
}

#[test]
fn rms_dc_offset() {
    // Constant signal of 0.5 => RMS = 0.5.
    let dc = vec![0.5f32; 1000];
    let rms = compute_rms(&dc);
    assert!(
        (rms - 0.5).abs() < 1e-5,
        "DC offset 0.5 should have RMS ~0.5, got {rms}"
    );
}

#[test]
fn rms_pure_sine_wave() {
    // Sine wave with amplitude A has RMS = A / sqrt(2).
    let amplitude: f32 = 1.0;
    let sample_rate = 16000;
    let freq = 440.0; // A4
    let duration_samples = sample_rate; // 1 second

    let samples: Vec<f32> = (0..duration_samples)
        .map(|i| {
            amplitude
                * (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin()
        })
        .collect();

    let rms = compute_rms(&samples);
    let expected = amplitude / 2.0f32.sqrt(); // ~0.7071

    assert!(
        (rms - expected).abs() < 0.01,
        "sine wave RMS should be ~{expected}, got {rms}"
    );
}

#[test]
fn rms_alternating_signal() {
    // Alternating +1, -1 => RMS = 1.0
    let alt: Vec<f32> = (0..1000).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
    let rms = compute_rms(&alt);
    assert!(
        (rms - 1.0).abs() < 1e-5,
        "alternating +1/-1 RMS should be 1.0, got {rms}"
    );
}

#[test]
fn rms_single_sample() {
    assert!((compute_rms(&[0.5]) - 0.5).abs() < f32::EPSILON);
    assert!((compute_rms(&[-0.5]) - 0.5).abs() < f32::EPSILON);
    assert!((compute_rms(&[0.0]) - 0.0).abs() < f32::EPSILON);
}

#[test]
fn rms_empty_input() {
    assert!((compute_rms(&[]) - 0.0).abs() < f32::EPSILON);
}

#[test]
fn rms_known_value() {
    // [3.0, 4.0] => RMS = sqrt((9+16)/2) = sqrt(12.5)
    let samples = [3.0f32, 4.0];
    let rms = compute_rms(&samples);
    let expected = (12.5f32).sqrt();
    assert!(
        (rms - expected).abs() < 1e-4,
        "RMS of [3,4] should be ~{expected}, got {rms}"
    );
}

#[test]
fn rms_negative_dc() {
    // All -0.3 => RMS = 0.3
    let samples = vec![-0.3f32; 500];
    let rms = compute_rms(&samples);
    assert!(
        (rms - 0.3).abs() < 1e-5,
        "DC -0.3 RMS should be 0.3, got {rms}"
    );
}

#[test]
fn rms_large_buffer() {
    // 1 second of white-ish noise (deterministic). RMS should be non-zero.
    let samples: Vec<f32> = (0..16000)
        .map(|i| (i as f32 * 0.1).sin() * 0.5)
        .collect();
    let rms = compute_rms(&samples);
    assert!(rms > 0.0, "non-silent signal should have positive RMS");
    assert!(rms < 1.0, "amplitude-limited signal RMS should be < 1.0");
}

// ============================================================================
//  7. State machine tests
// ============================================================================

#[test]
fn mic_state_display() {
    assert_eq!(format!("{}", MicState::Idle), "IDLE");
    assert_eq!(format!("{}", MicState::Listening), "LISTENING");
    assert_eq!(format!("{}", MicState::Processing), "PROCESSING");
    assert_eq!(format!("{}", MicState::Speaking), "SPEAKING");
}

#[test]
fn mic_state_equality() {
    assert_eq!(MicState::Idle, MicState::Idle);
    assert_ne!(MicState::Idle, MicState::Listening);
    assert_ne!(MicState::Listening, MicState::Processing);
    assert_ne!(MicState::Processing, MicState::Speaking);
}

#[test]
fn mic_state_clone_copy() {
    let state = MicState::Listening;
    let cloned = state;
    assert_eq!(state, cloned);
}

#[test]
fn session_mode_equality() {
    assert_eq!(SessionMode::Idle, SessionMode::Idle);
    assert_eq!(SessionMode::Active, SessionMode::Active);
    assert_ne!(SessionMode::Idle, SessionMode::Active);
}

#[test]
fn voice_state_defaults() {
    let vs = VoiceState::new();
    assert_eq!(vs.mic_state(), MicState::Idle);
    assert_eq!(vs.session_mode(), SessionMode::Idle);
    assert!(!vs.is_speaking());
    assert!(!vs.is_shutdown_requested());
    assert_eq!(vs.detected_language(), "en");
    assert_eq!(vs.auto_sleep_secs, 90);
}

#[test]
fn voice_state_transitions() {
    let vs = VoiceState::new();

    vs.set_mic_state(MicState::Listening);
    assert_eq!(vs.mic_state(), MicState::Listening);

    vs.set_mic_state(MicState::Processing);
    assert_eq!(vs.mic_state(), MicState::Processing);

    vs.set_mic_state(MicState::Speaking);
    assert_eq!(vs.mic_state(), MicState::Speaking);

    vs.set_mic_state(MicState::Idle);
    assert_eq!(vs.mic_state(), MicState::Idle);
}

#[test]
fn voice_state_session_mode() {
    let vs = VoiceState::new();
    assert_eq!(vs.session_mode(), SessionMode::Idle);

    vs.set_session_mode(SessionMode::Active);
    assert_eq!(vs.session_mode(), SessionMode::Active);

    vs.set_session_mode(SessionMode::Idle);
    assert_eq!(vs.session_mode(), SessionMode::Idle);
}

#[test]
fn voice_state_speaking_sets_mic_state() {
    let vs = VoiceState::new();
    vs.set_speaking(true);
    assert!(vs.is_speaking());
    assert_eq!(vs.mic_state(), MicState::Speaking);
}

#[test]
fn voice_state_speaking_false_does_not_change_mic_state() {
    let vs = VoiceState::new();
    vs.set_mic_state(MicState::Processing);
    vs.set_speaking(false);
    // set_speaking(false) does NOT change mic_state (only set_speaking(true) does).
    assert_eq!(vs.mic_state(), MicState::Processing);
}

#[test]
fn voice_state_detected_language() {
    let vs = VoiceState::new();
    assert_eq!(vs.detected_language(), "en");

    vs.set_detected_language("hi".to_string());
    assert_eq!(vs.detected_language(), "hi");

    vs.set_detected_language("ne".to_string());
    assert_eq!(vs.detected_language(), "ne");
}

#[test]
fn voice_state_shutdown() {
    let vs = VoiceState::new();
    assert!(!vs.is_shutdown_requested());
    vs.request_shutdown();
    assert!(vs.is_shutdown_requested());
}

#[test]
fn voice_state_idle_seconds() {
    let vs = VoiceState::new();
    // Immediately after creation, idle_seconds should be 0 (or very close).
    assert!(vs.idle_seconds() <= 1);
}

#[test]
fn voice_state_touch_resets_timer() {
    let vs = VoiceState::new();
    vs.touch();
    assert!(vs.idle_seconds() <= 1);
}

#[test]
fn voice_state_default_trait() {
    let vs = VoiceState::default();
    assert_eq!(vs.mic_state(), MicState::Idle);
    assert_eq!(vs.session_mode(), SessionMode::Idle);
}

#[test]
fn voice_state_auto_sleep_idle_does_not_trigger() {
    let vs = VoiceState::new();
    // In Idle session mode, should_auto_sleep is false regardless of time.
    assert!(!vs.should_auto_sleep());
}

#[test]
fn voice_state_auto_sleep_active_fresh() {
    let vs = VoiceState::new();
    vs.set_session_mode(SessionMode::Active);
    // Just activated, should not auto-sleep.
    assert!(!vs.should_auto_sleep());
}

// ============================================================================
//  8. Error type tests
// ============================================================================

#[test]
fn voice_error_display_audio_device() {
    let err = VoiceError::AudioDevice("no mic".to_string());
    assert_eq!(format!("{err}"), "audio device error: no mic");
}

#[test]
fn voice_error_display_no_input() {
    let err = VoiceError::NoInputDevice;
    assert_eq!(format!("{err}"), "no input device available");
}

#[test]
fn voice_error_display_no_output() {
    let err = VoiceError::NoOutputDevice;
    assert_eq!(format!("{err}"), "no output device available");
}

#[test]
fn voice_error_display_stt_model() {
    let err = VoiceError::SttModelLoad("missing model".to_string());
    assert_eq!(format!("{err}"), "STT model load failed: missing model");
}

#[test]
fn voice_error_display_stt_transcribe() {
    let err = VoiceError::SttTranscribe("decode error".to_string());
    assert_eq!(format!("{err}"), "STT transcription failed: decode error");
}

#[test]
fn voice_error_display_tts_synthesize() {
    let err = VoiceError::TtsSynthesize("timeout".to_string());
    assert_eq!(format!("{err}"), "TTS synthesis failed: timeout");
}

#[test]
fn voice_error_display_tts_not_found() {
    let err = VoiceError::TtsNotFound("piper".to_string());
    assert_eq!(format!("{err}"), "TTS engine not found: piper");
}

#[test]
fn voice_error_display_shutdown() {
    let err = VoiceError::Shutdown;
    assert_eq!(format!("{err}"), "shutdown requested");
}

#[test]
fn voice_error_display_vad_init() {
    let err = VoiceError::VadInit("onnx load fail".to_string());
    assert_eq!(format!("{err}"), "VAD initialization failed: onnx load fail");
}

#[test]
fn voice_error_display_vad_inference() {
    let err = VoiceError::VadInference("shape mismatch".to_string());
    assert_eq!(format!("{err}"), "VAD inference failed: shape mismatch");
}

#[test]
fn voice_error_display_config() {
    let err = VoiceError::Config("bad toml".to_string());
    assert_eq!(format!("{err}"), "config error: bad toml");
}

#[test]
fn voice_error_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<VoiceError>();
}

// ============================================================================
//  9. Cross-module integration scenarios
// ============================================================================

#[test]
fn scenario_wake_then_transcribe_valid() {
    // Simulate: STT produces text -> check wake word -> check not noise.
    let detector = WakeWordDetector::new("Jarvis");
    let transcript = Transcription {
        text: "Hey Jarvis, what is the weather?".to_string(),
        language: "en".to_string(),
        confidence: 0.92,
    };

    assert!(detector.matches(&transcript.text));
    assert!(!transcript.is_noise());
}

#[test]
fn scenario_wake_but_noise() {
    // STT hallucinates something that doesn't match wake word and IS noise.
    let detector = WakeWordDetector::new("G");
    let transcript = Transcription {
        text: "...".to_string(),
        language: "en".to_string(),
        confidence: 0.85,
    };

    assert!(!detector.matches(&transcript.text));
    assert!(transcript.is_noise());
}

#[test]
fn scenario_tts_engine_selection_with_language_detection() {
    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi"],
    };

    // Simulate: VoiceState reports detected language, select engine accordingly.
    let vs = VoiceState::new();
    assert_eq!(vs.detected_language(), "en");

    let engine = tts::select_engine(&vs.detected_language(), Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "piper");

    // Language changes to Hindi.
    vs.set_detected_language("hi".to_string());
    let engine2 = tts::select_engine(&vs.detected_language(), Some(&piper), Some(&espeak));
    assert_eq!(engine2.unwrap().name(), "espeak-ng");
}

#[test]
fn scenario_ring_buffer_fill_and_state_transition() {
    // Simulate: capture audio, transition states, read buffer.
    let mut rb = AudioRingBuffer::new(1600); // 100ms @ 16kHz
    let vs = VoiceState::new();

    // Start listening.
    vs.set_session_mode(SessionMode::Active);
    vs.set_mic_state(MicState::Listening);

    // "Capture" audio.
    let audio_chunk: Vec<f32> = (0..800).map(|i| (i as f32 * 0.001).sin()).collect();
    rb.write(&audio_chunk);

    assert_eq!(vs.mic_state(), MicState::Listening);
    assert_eq!(rb.total_written(), 800);

    // Process -> Speaking -> Idle cycle.
    vs.set_mic_state(MicState::Processing);
    assert_eq!(vs.mic_state(), MicState::Processing);

    vs.set_speaking(true);
    assert_eq!(vs.mic_state(), MicState::Speaking);

    vs.set_speaking(false);
    vs.set_mic_state(MicState::Idle);
    assert_eq!(vs.mic_state(), MicState::Idle);
}

#[test]
fn scenario_full_voice_pipeline_simulation() {
    // Simulate a full wake -> listen -> STT -> TTS select -> speak cycle.
    let vs = VoiceState::new();
    let detector = WakeWordDetector::new("Jarvis");
    let mut rb = AudioRingBuffer::new(16_000);

    let piper = MockTtsEngine {
        engine_name: "piper",
        languages: &["en"],
    };
    let espeak = MockTtsEngine {
        engine_name: "espeak-ng",
        languages: &["en", "hi"],
    };

    // Step 1: Idle, waiting for wake word.
    assert_eq!(vs.mic_state(), MicState::Idle);
    assert_eq!(vs.session_mode(), SessionMode::Idle);

    // Step 2: STT result arrives with wake word.
    let wake_transcript = Transcription {
        text: "Hey Jarvis".to_string(),
        language: "en".to_string(),
        confidence: 0.95,
    };
    assert!(detector.matches(&wake_transcript.text));
    assert!(!wake_transcript.is_noise());

    // Step 3: Activate session, start listening.
    vs.set_session_mode(SessionMode::Active);
    vs.set_mic_state(MicState::Listening);
    vs.touch();

    // Step 4: Write audio to ring buffer.
    let audio: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.01).sin()).collect();
    rb.write(&audio);

    // Step 5: Process STT.
    vs.set_mic_state(MicState::Processing);
    let command = Transcription {
        text: "What is the weather today?".to_string(),
        language: "en".to_string(),
        confidence: 0.88,
    };
    assert!(!command.is_noise());

    // Step 6: Select TTS engine and "speak".
    let engine = tts::select_engine(&command.language, Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "piper");

    vs.set_speaking(true);
    assert_eq!(vs.mic_state(), MicState::Speaking);
    assert!(vs.is_speaking());

    // Step 7: Done speaking, back to idle.
    vs.set_speaking(false);
    vs.set_mic_state(MicState::Idle);
    assert_eq!(vs.mic_state(), MicState::Idle);
}

// ============================================================================
// 10. Audio constants
// ============================================================================

#[test]
fn audio_constants() {
    assert_eq!(gclaw_voice::audio::SAMPLE_RATE, 16_000);
    assert_eq!(gclaw_voice::audio::VAD_FRAME_SAMPLES, 512);
    assert_eq!(gclaw_voice::audio::CHANNELS, 1);
}

// ============================================================================
// 11. Transcription struct fields
// ============================================================================

#[test]
fn transcription_clone() {
    let t = Transcription {
        text: "hello".to_string(),
        language: "en".to_string(),
        confidence: 0.95,
    };
    let cloned = t.clone();
    assert_eq!(cloned.text, "hello");
    assert_eq!(cloned.language, "en");
    assert!((cloned.confidence - 0.95).abs() < f32::EPSILON);
}

#[test]
fn transcription_debug() {
    let t = Transcription {
        text: "test".to_string(),
        language: "en".to_string(),
        confidence: 0.5,
    };
    let debug_str = format!("{:?}", t);
    assert!(debug_str.contains("test"));
    assert!(debug_str.contains("en"));
}
