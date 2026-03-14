/// VoiceShell — main orchestrator for the voice pipeline.
///
/// Mirrors: assistant_loop.py IDLE/ACTIVE state machine + speech.py functions.
///
/// Main loop:
///   IDLE mode:  VAD → STT → wake word check → transition to ACTIVE
///   ACTIVE mode: VAD → STT → send to brain via IPC; receive Speak/SpeakInterruptible
///
/// All audio, VAD, STT, TTS, and IPC are handled here.
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use gclaw_ipc::protocol::{Message, UserSpeech, BargeIn, SpeakRequest};
use gclaw_ipc::transport::IpcTransport;

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::audio::{SAMPLE_RATE, VAD_FRAME_SAMPLES};
use crate::bargein::{BargeInController, BargeInResult};
use crate::stt::{Transcription, WhisperStt};
use crate::tts::{self, TtsEngine, TtsAudio};
use crate::vad::{SileroVad, VadDecision};
use crate::wake::WakeWordDetector;
use crate::state::{MicState, SessionMode, VoiceState};

/// Configuration for the voice shell.
pub struct VoiceShellConfig {
    /// Path to Silero VAD ONNX model.
    pub vad_model_path: String,
    /// Path to Whisper GGML model.
    pub whisper_model_path: String,
    /// Path to Piper binary (optional).
    pub piper_binary_path: Option<String>,
    /// Path to Piper voice model (optional).
    pub piper_model_path: Option<String>,
    /// AI name for wake word detection.
    pub ai_name: String,
    /// Language hint for STT (None = auto-detect).
    pub language: Option<String>,
    /// IPC port for TCP transport (Windows).
    pub ipc_port: u16,
}

impl Default for VoiceShellConfig {
    fn default() -> Self {
        Self {
            vad_model_path: "models/silero_vad.onnx".into(),
            whisper_model_path: "models/ggml-base.bin".into(),
            piper_binary_path: None,
            piper_model_path: None,
            ai_name: "G".into(),
            language: None,
            ipc_port: gclaw_ipc::transport::VOICE_TCP_PORT,
        }
    }
}

/// The voice shell — main entry point for gclaw-voice.
pub struct VoiceShell {
    config: VoiceShellConfig,
    state: Arc<VoiceState>,
}

impl VoiceShell {
    pub fn new(config: VoiceShellConfig) -> Self {
        Self {
            config,
            state: Arc::new(VoiceState::new()),
        }
    }

    /// Run the voice shell main loop.
    ///
    /// This is the top-level entry point. It:
    ///   1. Initializes all subsystems (audio, VAD, STT, TTS)
    ///   2. Connects to the brain via IPC
    ///   3. Runs the IDLE/ACTIVE state machine
    pub async fn run(&self) -> Result<()> {
        info!("gclaw-voice starting...");

        // Initialize subsystems.
        let capture = AudioCapture::new(30.0).context("init audio capture")?;
        let playback = AudioPlayback::new().context("init audio playback")?;
        let mut vad = SileroVad::new(&self.config.vad_model_path).context("init VAD")?;
        let stt = WhisperStt::new(&self.config.whisper_model_path).context("init STT")?;
        let wake_detector = WakeWordDetector::new(&self.config.ai_name);
        let bargein = BargeInController::new();

        // Initialize TTS engines.
        let piper: Option<Box<dyn TtsEngine>> =
            if let (Some(bin), Some(model)) = (&self.config.piper_binary_path, &self.config.piper_model_path) {
                match crate::tts::piper::PiperTts::new(bin.as_ref(), model.as_ref()) {
                    Ok(p) => {
                        info!("piper TTS initialized");
                        Some(Box::new(p))
                    }
                    Err(e) => {
                        warn!("piper init failed: {e} — falling back to espeak");
                        None
                    }
                }
            } else {
                None
            };

        let espeak: Option<Box<dyn TtsEngine>> =
            match crate::tts::espeak::EspeakTts::new("en") {
                Ok(e) => {
                    info!("espeak-ng TTS initialized");
                    Some(Box::new(e))
                }
                Err(e) => {
                    warn!("espeak-ng not available: {e}");
                    None
                }
            };

        // Connect to brain IPC.
        info!("connecting to brain IPC on port {}...", self.config.ipc_port);
        let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", self.config.ipc_port))
            .await
            .context("connect to brain IPC")?;
        let mut ipc = IpcTransport::new(stream);

        // Send Ready message.
        ipc.send(&Message::Ready).await?;
        info!("connected to brain, voice shell ready");

        // Main loop.
        let mut accumulated_audio: Vec<f32> = Vec::new();
        let mut frame_buf = vec![0.0f32; VAD_FRAME_SAMPLES];

        loop {
            if self.state.is_shutdown_requested() {
                info!("shutdown requested");
                break;
            }

            match self.state.session_mode() {
                SessionMode::Idle => {
                    // IDLE: Listen for wake word.
                    self.state.set_mic_state(MicState::Idle);

                    // Read latest audio frame for VAD.
                    let samples = capture.read_last_samples(VAD_FRAME_SAMPLES);
                    if samples.len() < VAD_FRAME_SAMPLES {
                        tokio::time::sleep(Duration::from_millis(32)).await;
                        continue;
                    }
                    frame_buf.copy_from_slice(&samples);

                    match vad.process_frame(&frame_buf, &mut accumulated_audio, capture.total_samples())? {
                        VadDecision::SpeechEnd(segment) => {
                            // Got speech — run STT and check for wake word.
                            self.state.set_mic_state(MicState::Processing);
                            let transcription = stt.transcribe(&segment.audio)?;

                            if !transcription.is_noise() && wake_detector.matches(&transcription.text) {
                                info!("wake word detected: \"{}\"", transcription.text);
                                self.state.set_session_mode(SessionMode::Active);
                                ipc.send(&Message::WakeWordDetected).await?;
                            }
                        }
                        _ => {
                            // Still waiting — small sleep to avoid busy loop.
                            tokio::time::sleep(Duration::from_millis(32)).await;
                        }
                    }
                }

                SessionMode::Active => {
                    // ACTIVE: Listen for user speech and handle brain commands.
                    self.state.set_mic_state(MicState::Listening);

                    // Check auto-sleep timeout (90s).
                    if self.state.should_auto_sleep() {
                        info!("auto-sleep: {}s idle", self.state.idle_seconds());
                        self.state.set_session_mode(SessionMode::Idle);
                        vad.reset();
                        continue;
                    }

                    // Check for incoming IPC messages (non-blocking).
                    tokio::select! {
                        // Listen for speech via VAD.
                        _ = tokio::time::sleep(Duration::from_millis(32)) => {
                            let samples = capture.read_last_samples(VAD_FRAME_SAMPLES);
                            if samples.len() >= VAD_FRAME_SAMPLES {
                                frame_buf.copy_from_slice(&samples);

                                match vad.process_frame(&frame_buf, &mut accumulated_audio, capture.total_samples())? {
                                    VadDecision::SpeechEnd(segment) => {
                                        self.state.set_mic_state(MicState::Processing);
                                        let transcription = stt.transcribe(&segment.audio)?;

                                        if !transcription.is_noise() {
                                            self.state.touch();
                                            self.state.set_detected_language(transcription.language.clone());

                                            ipc.send(&Message::UserSpeech(UserSpeech {
                                                text: transcription.text,
                                                language: transcription.language,
                                                confidence: transcription.confidence,
                                            })).await?;
                                        }

                                        self.state.set_mic_state(MicState::Listening);
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // Handle brain commands.
                        msg = ipc.recv() => {
                            match msg? {
                                Some(Message::Speak(req)) => {
                                    self.state.set_mic_state(MicState::Speaking);
                                    self.state.set_speaking(true);

                                    let lang = self.state.detected_language();
                                    if let Some(engine) = tts::select_engine(
                                        &lang,
                                        piper.as_deref(),
                                        espeak.as_deref(),
                                    ) {
                                        match engine.synthesize(&req.text) {
                                            Ok(audio) => {
                                                let _ = playback.play_samples(&audio.samples, audio.sample_rate);
                                            }
                                            Err(e) => warn!("TTS failed: {e}"),
                                        }
                                    } else {
                                        warn!("no TTS engine available");
                                    }

                                    self.state.set_speaking(false);
                                    self.state.set_mic_state(MicState::Listening);
                                    self.state.touch();
                                }

                                Some(Message::SpeakInterruptible(req)) => {
                                    self.state.set_mic_state(MicState::Speaking);
                                    self.state.set_speaking(true);

                                    let lang = self.state.detected_language();
                                    if let Some(engine) = tts::select_engine(
                                        &lang,
                                        piper.as_deref(),
                                        espeak.as_deref(),
                                    ) {
                                        match engine.synthesize(&req.text) {
                                            Ok(audio) => {
                                                match bargein.speak_interruptible(
                                                    &audio, &playback, &capture, &mut vad, &stt,
                                                ) {
                                                    Ok(BargeInResult::Interrupted(text)) => {
                                                        ipc.send(&Message::BargeIn(BargeIn { text })).await?;
                                                    }
                                                    Ok(BargeInResult::Completed) => {}
                                                    Err(e) => warn!("barge-in error: {e}"),
                                                }
                                            }
                                            Err(e) => warn!("TTS failed: {e}"),
                                        }
                                    }

                                    self.state.set_speaking(false);
                                    self.state.set_mic_state(MicState::Listening);
                                    self.state.touch();
                                }

                                Some(Message::StopSpeaking) => {
                                    bargein.stop();
                                    self.state.set_speaking(false);
                                    self.state.set_mic_state(MicState::Listening);
                                }

                                Some(Message::SetMicState(req)) => {
                                    let new_state = match req.state {
                                        gclaw_ipc::MicState::Idle => MicState::Idle,
                                        gclaw_ipc::MicState::Listening => MicState::Listening,
                                        gclaw_ipc::MicState::Processing => MicState::Processing,
                                        gclaw_ipc::MicState::Speaking => MicState::Speaking,
                                    };
                                    self.state.set_mic_state(new_state);
                                }

                                Some(Message::Configure(cfg)) => {
                                    if let Some(name) = cfg.ai_name {
                                        info!("reconfigured AI name to: {name}");
                                        // Wake detector would need rebuild — for now just log.
                                    }
                                    if let Some(lang) = cfg.language {
                                        self.state.set_detected_language(lang);
                                    }
                                }

                                Some(Message::Shutdown) => {
                                    info!("shutdown command received");
                                    self.state.request_shutdown();
                                    break;
                                }

                                Some(Message::Ping) => {
                                    ipc.send(&Message::Pong).await?;
                                }

                                None => {
                                    warn!("brain disconnected");
                                    break;
                                }

                                Some(other) => {
                                    debug!("ignoring unexpected message: {:?}", std::mem::discriminant(&other));
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("gclaw-voice shutting down");
        Ok(())
    }

    /// Get the current state (for external queries).
    pub fn state(&self) -> &Arc<VoiceState> {
        &self.state
    }
}
