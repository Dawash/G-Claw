/// Voice shell state — mirrors core/state.py AudioState + SessionState.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Microphone state machine (mirrors AudioState.mic_state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    /// Waiting for wake word or input.
    Idle,
    /// Actively capturing user speech.
    Listening,
    /// Running STT on captured audio.
    Processing,
    /// Playing TTS output.
    Speaking,
}

impl std::fmt::Display for MicState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "IDLE"),
            Self::Listening => write!(f, "LISTENING"),
            Self::Processing => write!(f, "PROCESSING"),
            Self::Speaking => write!(f, "SPEAKING"),
        }
    }
}

/// Session mode (IDLE = waiting for wake word, ACTIVE = listening for commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    Idle,
    Active,
}

/// Thread-safe voice shell state.
pub struct VoiceState {
    mic_state: Mutex<MicState>,
    session_mode: Mutex<SessionMode>,
    is_speaking: AtomicBool,
    last_activity: Mutex<Instant>,
    detected_language: Mutex<String>,
    shutdown_requested: AtomicBool,

    /// Auto-sleep timeout in seconds (mirrors SessionState.auto_sleep_seconds = 90).
    pub auto_sleep_secs: u64,
}

impl VoiceState {
    pub fn new() -> Self {
        Self {
            mic_state: Mutex::new(MicState::Idle),
            session_mode: Mutex::new(SessionMode::Idle),
            is_speaking: AtomicBool::new(false),
            last_activity: Mutex::new(Instant::now()),
            detected_language: Mutex::new("en".into()),
            shutdown_requested: AtomicBool::new(false),
            auto_sleep_secs: 90,
        }
    }

    pub fn mic_state(&self) -> MicState {
        *self.mic_state.lock().unwrap()
    }

    pub fn set_mic_state(&self, state: MicState) {
        *self.mic_state.lock().unwrap() = state;
    }

    pub fn session_mode(&self) -> SessionMode {
        *self.session_mode.lock().unwrap()
    }

    pub fn set_session_mode(&self, mode: SessionMode) {
        let mut m = self.session_mode.lock().unwrap();
        *m = mode;
        if mode == SessionMode::Active {
            self.touch();
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.is_speaking.load(Ordering::Relaxed)
    }

    pub fn set_speaking(&self, speaking: bool) {
        self.is_speaking.store(speaking, Ordering::Relaxed);
        if speaking {
            self.set_mic_state(MicState::Speaking);
        }
    }

    /// Update last activity timestamp (call on any user interaction).
    pub fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }

    /// Seconds since last activity.
    pub fn idle_seconds(&self) -> u64 {
        self.last_activity.lock().unwrap().elapsed().as_secs()
    }

    /// Should auto-sleep? (mirrors 90s timeout from assistant_loop.py)
    pub fn should_auto_sleep(&self) -> bool {
        self.session_mode() == SessionMode::Active && self.idle_seconds() > self.auto_sleep_secs
    }

    pub fn detected_language(&self) -> String {
        self.detected_language.lock().unwrap().clone()
    }

    pub fn set_detected_language(&self, lang: String) {
        *self.detected_language.lock().unwrap() = lang;
    }

    pub fn request_shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
    }

    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
    }
}

impl Default for VoiceState {
    fn default() -> Self {
        Self::new()
    }
}
