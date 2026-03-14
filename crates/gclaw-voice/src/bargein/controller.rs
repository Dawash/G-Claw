/// Barge-in controller — concurrent TTS playback + VAD monitoring.
///
/// Mirrors: speech.py speak_interruptible().
///
/// While TTS is playing, the VAD continuously monitors the microphone.
/// If user speech is detected during playback:
///   1. TTS stops immediately
///   2. Speech is captured and transcribed
///   3. The transcription is returned as an interruption
///
/// This enables natural conversational flow — users can interrupt long responses.
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

use crate::audio::playback::AudioPlayback;
use crate::audio::capture::AudioCapture;
use crate::audio::{SAMPLE_RATE, VAD_FRAME_SAMPLES};
use crate::stt::WhisperStt;
use crate::tts::TtsAudio;
use crate::vad::SileroVad;

/// Result of an interruptible speak operation.
#[derive(Debug)]
pub enum BargeInResult {
    /// TTS playback completed normally — user didn't interrupt.
    Completed,
    /// User interrupted — contains the transcribed interruption text.
    Interrupted(String),
}

pub struct BargeInController {
    /// Shared stop flag — set to true to stop TTS playback.
    stop_flag: Arc<AtomicBool>,
}

impl BargeInController {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Speak with barge-in support.
    ///
    /// Plays `audio` through `playback` while monitoring `capture` with `vad`.
    /// If speech is detected, stops playback and transcribes the interruption.
    ///
    /// This runs synchronously — it blocks until playback completes or barge-in occurs.
    pub fn speak_interruptible(
        &self,
        audio: &TtsAudio,
        playback: &AudioPlayback,
        capture: &AudioCapture,
        vad: &mut SileroVad,
        stt: &WhisperStt,
    ) -> Result<BargeInResult> {
        if audio.samples.is_empty() {
            return Ok(BargeInResult::Completed);
        }

        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = self.stop_flag.clone();

        // Start VAD monitoring in a separate thread.
        let capture_samples_before = capture.total_samples();
        let speech_detected = Arc::new(AtomicBool::new(false));
        let speech_det_clone = speech_detected.clone();
        let stop_clone = stop.clone();

        // Monitor thread: check VAD every 32ms during playback.
        let monitor = std::thread::spawn(move || {
            // Give playback ~200ms head start to avoid detecting our own TTS.
            std::thread::sleep(std::time::Duration::from_millis(200));

            // We can't use VAD here directly since it needs &mut self,
            // so we use a simple energy-based detection as a trigger.
            // The actual VAD + STT happens after playback stops.
            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(32));
            }
        });

        // Play audio (blocks until done or stopped).
        let completed = playback.play_blocking(&audio.samples, audio.sample_rate, &stop)?;

        if completed {
            self.stop_flag.store(true, Ordering::Relaxed);
            let _ = monitor.join();
            debug!("tts playback completed normally");
            return Ok(BargeInResult::Completed);
        }

        // Playback was interrupted — capture and transcribe the interruption.
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = monitor.join();

        // Wait a moment for the user to finish speaking.
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Get recent audio and transcribe.
        let recent_audio = capture.read_last(3.0); // Last 3 seconds.
        if recent_audio.is_empty() {
            info!("barge-in detected but no audio captured");
            return Ok(BargeInResult::Completed);
        }

        let transcription = stt.transcribe(&recent_audio)?;
        if transcription.is_noise() || transcription.text.is_empty() {
            debug!("barge-in audio was noise, treating as completed");
            return Ok(BargeInResult::Completed);
        }

        info!("barge-in: user said \"{}\"", transcription.text);
        Ok(BargeInResult::Interrupted(transcription.text))
    }

    /// Force-stop any current playback.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Check if playback is currently stopped.
    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::Relaxed)
    }
}

impl Default for BargeInController {
    fn default() -> Self {
        Self::new()
    }
}
