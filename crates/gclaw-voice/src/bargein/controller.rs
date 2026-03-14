/// Barge-in controller — concurrent TTS playback + speech monitoring.
///
/// Mirrors: speech.py speak_interruptible().
///
/// While TTS is playing, the controller monitors the microphone using
/// energy-based speech detection (RMS threshold). When user speech is
/// detected during playback:
///   1. TTS stops immediately
///   2. Short silence to let user finish speaking
///   3. Recent audio is captured and transcribed
///   4. The transcription is returned as an interruption
///
/// Note: We use energy-based detection (not Silero VAD) during playback
/// because VAD requires &mut self which can't be shared across threads.
/// Energy detection is sufficient for barge-in triggering — the full
/// VAD+STT pipeline runs post-interruption.
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::audio::VAD_FRAME_SAMPLES;
#[cfg(feature = "full")]
use crate::stt::WhisperStt;
use crate::tts::TtsAudio;
#[cfg(feature = "full")]
use crate::vad::SileroVad;

/// RMS energy threshold for barge-in detection.
/// Typical speech is 0.02-0.1 RMS; background noise is 0.001-0.01.
/// We use a relatively high threshold to avoid false triggers from TTS
/// audio leaking into the microphone.
const BARGEIN_RMS_THRESHOLD: f32 = 0.03;

/// Number of consecutive high-energy frames needed to trigger barge-in.
/// 3 frames * 32ms = ~100ms of sustained speech.
const BARGEIN_CONSECUTIVE_FRAMES: u32 = 3;

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
    /// Plays `audio` through `playback` while monitoring `capture` for speech.
    /// Uses energy-based detection to trigger interruption, then transcribes
    /// the captured audio post-interruption.
    #[cfg(feature = "full")]
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

        self.stop_flag.store(false, Ordering::SeqCst);
        let stop = self.stop_flag.clone();
        let stop_for_monitor = stop.clone();

        // Spawn energy-based speech monitor thread.
        // This thread reads from the ring buffer and computes RMS energy.
        // If sustained speech energy is detected, it sets the stop flag
        // to interrupt TTS playback.
        let capture_ptr = capture as *const AudioCapture;
        // SAFETY: capture lives for the duration of this function call,
        // and the monitor thread is joined before we return.
        let monitor = std::thread::spawn(move || {
            // Give playback ~250ms head start to avoid detecting our own TTS
            // via speaker-to-mic acoustic feedback.
            std::thread::sleep(std::time::Duration::from_millis(250));

            let capture_ref = unsafe { &*capture_ptr };
            let mut consecutive_high = 0u32;

            loop {
                if stop_for_monitor.load(Ordering::SeqCst) {
                    break;
                }

                // Read a frame-sized chunk of recent audio.
                let samples = capture_ref.read_last_samples(VAD_FRAME_SAMPLES);
                if samples.len() >= VAD_FRAME_SAMPLES {
                    // Compute RMS energy.
                    let rms = compute_rms(&samples);

                    if rms > BARGEIN_RMS_THRESHOLD {
                        consecutive_high += 1;
                        if consecutive_high >= BARGEIN_CONSECUTIVE_FRAMES {
                            debug!("barge-in: speech energy detected (rms={rms:.4}, consecutive={consecutive_high})");
                            stop_for_monitor.store(true, Ordering::SeqCst);
                            break;
                        }
                    } else {
                        consecutive_high = 0;
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(32));
            }
        });

        // Play audio (blocks until done or stopped by monitor).
        let completed = playback.play_blocking(&audio.samples, audio.sample_rate, &stop)?;

        // Ensure monitor stops if playback completed normally.
        self.stop_flag.store(true, Ordering::SeqCst);
        let _ = monitor.join();

        if completed {
            debug!("tts playback completed normally");
            return Ok(BargeInResult::Completed);
        }

        // Playback was interrupted — wait for user to finish speaking, then transcribe.
        info!("barge-in triggered, capturing user speech");
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Capture recent audio (last 3 seconds covers most interruptions).
        let recent_audio = capture.read_last(3.0);
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
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Check if playback is currently stopped.
    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }
}

/// Compute RMS (root-mean-square) energy of audio samples.
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

impl Default for BargeInController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_silence() {
        let silence = vec![0.0f32; 512];
        assert!(compute_rms(&silence) < 0.001);
    }

    #[test]
    fn rms_speech() {
        // Simulate speech-level signal (~0.05 RMS).
        let speech: Vec<f32> = (0..512)
            .map(|i| 0.07 * (i as f32 * 0.1).sin())
            .collect();
        let rms = compute_rms(&speech);
        assert!(rms > 0.01 && rms < 0.1);
    }

    #[test]
    fn rms_empty() {
        assert_eq!(compute_rms(&[]), 0.0);
    }
}
