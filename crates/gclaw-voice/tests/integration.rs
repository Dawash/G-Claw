/// Integration tests for gclaw-voice (no "full" feature required).
use gclaw_voice::audio::capture::AudioRingBuffer;
use gclaw_voice::stt::Transcription;
use gclaw_voice::state::{MicState, SessionMode, VoiceState};
use gclaw_voice::tts::{self, TtsAudio, TtsEngine};
use gclaw_voice::wake::WakeWordDetector;

// ---------------------------------------------------------------
// Ring Buffer
// ---------------------------------------------------------------

#[test]
fn ring_buffer_capacity_one() {
    let mut rb = AudioRingBuffer::new(1);
    rb.write(&[1.0]);
    assert_eq!(rb.read_last(1), vec![1.0]);
    rb.write(&[2.0]);
    assert_eq!(rb.read_last(1), vec![2.0]);
}

#[test]
fn ring_buffer_exact_capacity() {
    let mut rb = AudioRingBuffer::new(4);
    rb.write(&[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(rb.read_last(4), vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn ring_buffer_read_zero() {
    let mut rb = AudioRingBuffer::new(10);
    rb.write(&[1.0, 2.0]);
    assert_eq!(rb.read_last(0), Vec::<f32>::new());
}

#[test]
fn ring_buffer_stress() {
    let mut rb = AudioRingBuffer::new(1000);
    // Write 100K samples in chunks.
    for batch in 0..1000 {
        let chunk: Vec<f32> = (0..100).map(|i| (batch * 100 + i) as f32).collect();
        rb.write(&chunk);
    }
    // Last 5 samples should be 99995..99999.
    let last5 = rb.read_last(5);
    assert_eq!(last5, vec![99995.0, 99996.0, 99997.0, 99998.0, 99999.0]);
    assert_eq!(rb.total_written(), 100_000);
}

#[test]
fn ring_buffer_multiple_wraps() {
    let mut rb = AudioRingBuffer::new(3);
    for i in 0..20 {
        rb.write(&[i as f32]);
    }
    // Buffer of size 3 with 20 writes: last 3 should be 17, 18, 19.
    assert_eq!(rb.read_last(3), vec![17.0, 18.0, 19.0]);
}

// ---------------------------------------------------------------
// Transcription noise filtering
// ---------------------------------------------------------------

#[test]
fn noise_all_hallucinations() {
    let hallucinations = [
        "Thank you.", "thanks for watching.", "Thank you for watching.",
        "thanks for watching!", "The end.", "you", "Bye.", "bye!",
        "bye bye.", "...", "so", "i'm sorry.",
    ];
    for h in hallucinations {
        let t = Transcription { text: h.into(), language: "en".into(), confidence: 0.9 };
        assert!(t.is_noise(), "\"{h}\" should be noise");
    }
}

#[test]
fn noise_short_text() {
    assert!(Transcription { text: "a".into(), language: "en".into(), confidence: 0.9 }.is_noise());
    assert!(Transcription { text: "".into(), language: "en".into(), confidence: 0.9 }.is_noise());
    assert!(Transcription { text: " ".into(), language: "en".into(), confidence: 0.9 }.is_noise());
}

#[test]
fn noise_all_punctuation() {
    assert!(Transcription { text: "...!?".into(), language: "en".into(), confidence: 0.9 }.is_noise());
    assert!(Transcription { text: "---".into(), language: "en".into(), confidence: 0.9 }.is_noise());
}

#[test]
fn noise_low_confidence() {
    assert!(Transcription { text: "real words here".into(), language: "en".into(), confidence: 0.05 }.is_noise());
}

#[test]
fn not_noise_valid_speech() {
    let cases = [
        "hello world", "what time is it", "set a timer for 5 minutes",
        "play some music", "hey G what's the weather",
    ];
    for text in cases {
        let t = Transcription { text: text.into(), language: "en".into(), confidence: 0.8 };
        assert!(!t.is_noise(), "\"{text}\" should NOT be noise");
    }
}

#[test]
fn not_noise_unicode() {
    let t = Transcription { text: "こんにちは".into(), language: "ja".into(), confidence: 0.8 };
    assert!(!t.is_noise());
}

// ---------------------------------------------------------------
// Wake word detection
// ---------------------------------------------------------------

#[test]
fn wake_single_char_name() {
    let d = WakeWordDetector::new("G");
    assert!(d.matches("hey G"));
    assert!(d.matches("Hey G!"));
    assert!(d.matches("ok g"));
    assert!(d.matches("hey gee")); // Mishearing variant
    assert!(!d.matches("the weather is nice today"));
    assert!(!d.matches("set a timer for five minutes"));
}

#[test]
fn wake_medium_name() {
    let d = WakeWordDetector::new("Jarvis");
    assert!(d.matches("hey jarvis"));
    assert!(d.matches("JARVIS"));
    assert!(d.matches("ok jarvis what time is it"));
    assert!(!d.matches("the stock market crashed today"));
}

#[test]
fn wake_long_name() {
    let d = WakeWordDetector::new("Computer");
    assert!(d.matches("hey computer"));
    assert!(d.matches("ok computer"));
    assert!(d.matches("hello computer"));
    assert!(!d.matches("the weather is nice today"));
}

#[test]
fn wake_empty_input() {
    let d = WakeWordDetector::new("G");
    assert!(!d.matches(""));
    assert!(!d.matches("   "));
}

#[test]
fn wake_custom_variant() {
    let mut d = WakeWordDetector::new("G");
    d.add_variant("activate");
    assert!(d.matches("activate"));
}

// ---------------------------------------------------------------
// TTS engine selection
// ---------------------------------------------------------------

struct MockEngine {
    name: &'static str,
    langs: &'static [&'static str],
}

impl TtsEngine for MockEngine {
    fn synthesize(&self, _text: &str) -> anyhow::Result<TtsAudio> {
        Ok(TtsAudio { samples: vec![0.0; 100], sample_rate: 22050 })
    }
    fn name(&self) -> &str { self.name }
    fn supported_languages(&self) -> &[&str] { self.langs }
}

#[test]
fn tts_select_english_prefers_piper() {
    let piper = MockEngine { name: "piper", langs: &["en"] };
    let espeak = MockEngine { name: "espeak", langs: &["en", "hi"] };
    let engine = tts::select_engine("en", Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "piper");
}

#[test]
fn tts_select_hindi_prefers_espeak() {
    let piper = MockEngine { name: "piper", langs: &["en"] };
    let espeak = MockEngine { name: "espeak", langs: &["en", "hi"] };
    let engine = tts::select_engine("hi", Some(&piper), Some(&espeak));
    assert_eq!(engine.unwrap().name(), "espeak");
}

#[test]
fn tts_select_english_fallback_espeak() {
    let espeak = MockEngine { name: "espeak", langs: &["en"] };
    let engine = tts::select_engine("en", None, Some(&espeak));
    assert_eq!(engine.unwrap().name(), "espeak");
}

#[test]
fn tts_select_neither_available() {
    let engine = tts::select_engine("en", None, None);
    assert!(engine.is_none());
}

#[test]
fn tts_select_empty_language_defaults_to_piper() {
    let piper = MockEngine { name: "piper", langs: &["en"] };
    let engine = tts::select_engine("", Some(&piper), None);
    assert_eq!(engine.unwrap().name(), "piper");
}

// ---------------------------------------------------------------
// Voice state management
// ---------------------------------------------------------------

#[test]
fn state_initial_values() {
    let state = VoiceState::new();
    assert_eq!(state.mic_state(), MicState::Idle);
    assert_eq!(state.session_mode(), SessionMode::Idle);
    assert!(!state.is_speaking());
    assert!(!state.is_shutdown_requested());
    assert_eq!(state.detected_language(), "en");
}

#[test]
fn state_transitions() {
    let state = VoiceState::new();
    state.set_mic_state(MicState::Listening);
    assert_eq!(state.mic_state(), MicState::Listening);

    state.set_session_mode(SessionMode::Active);
    assert_eq!(state.session_mode(), SessionMode::Active);

    state.set_speaking(true);
    assert!(state.is_speaking());
    assert_eq!(state.mic_state(), MicState::Speaking);
}

#[test]
fn state_auto_sleep() {
    let state = VoiceState::new();
    state.set_session_mode(SessionMode::Active);
    // Just activated — shouldn't auto-sleep yet.
    assert!(!state.should_auto_sleep());

    // Idle mode should never trigger auto-sleep.
    state.set_session_mode(SessionMode::Idle);
    assert!(!state.should_auto_sleep());
}

#[test]
fn state_shutdown() {
    let state = VoiceState::new();
    assert!(!state.is_shutdown_requested());
    state.request_shutdown();
    assert!(state.is_shutdown_requested());
}

#[test]
fn state_language() {
    let state = VoiceState::new();
    state.set_detected_language("hi".into());
    assert_eq!(state.detected_language(), "hi");
    state.set_detected_language("ne".into());
    assert_eq!(state.detected_language(), "ne");
}

#[test]
fn state_touch_resets_idle_timer() {
    let state = VoiceState::new();
    state.set_session_mode(SessionMode::Active);
    state.touch();
    assert!(state.idle_seconds() < 2);
}

// ---------------------------------------------------------------
// MicState display
// ---------------------------------------------------------------

#[test]
fn mic_state_display() {
    assert_eq!(format!("{}", MicState::Idle), "IDLE");
    assert_eq!(format!("{}", MicState::Listening), "LISTENING");
    assert_eq!(format!("{}", MicState::Processing), "PROCESSING");
    assert_eq!(format!("{}", MicState::Speaking), "SPEAKING");
}
