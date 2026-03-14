/// Silero VAD via ONNX Runtime — neural voice activity detection.
///
/// Mirrors: speech.py Silero VAD integration.
/// Processes 512-sample frames (32ms @ 16kHz) and outputs speech probability.
///
/// The Silero VAD v5 model expects:
///   Input:  [1, chunk_size] f32 audio + [2, 1, 64] LSTM state + [1] sample rate
///   Output: [1, 1] probability + [2, 1, 64] new LSTM state
use anyhow::{Context, Result};
use ndarray::Array3;
use tracing::{debug, trace};

use super::{SpeechSegment, VadDecision};
use crate::audio::VAD_FRAME_SAMPLES;

/// Speech probability threshold (above = speech, below = silence).
const SPEECH_THRESHOLD: f32 = 0.5;

/// Minimum speech duration in frames before we accept it (debounce).
/// 5 frames * 32ms = 160ms minimum utterance.
const MIN_SPEECH_FRAMES: u32 = 5;

/// Maximum silence frames within speech before we call SpeechEnd.
/// 15 frames * 32ms = 480ms pause threshold (mirrors speech.py pause_threshold = 0.5s).
const MAX_SILENCE_FRAMES: u32 = 15;

/// Maximum speech duration in frames (60s safety cap).
const MAX_SPEECH_FRAMES: u32 = 60 * 1000 / 32;

pub struct SileroVad {
    session: ort::Session,
    /// LSTM hidden state [2, 1, 64] — persists across frames.
    h_state: Array3<f32>,
    /// LSTM cell state [2, 1, 64].
    c_state: Array3<f32>,
    /// Current state tracking.
    state: VadInternalState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum VadInternalState {
    /// Waiting for speech.
    WaitingForSpeech,
    /// Speech detected, accumulating.
    InSpeech {
        speech_frames: u32,
        silence_frames: u32,
        start_sample: u64,
    },
}

impl SileroVad {
    /// Load the Silero VAD ONNX model.
    ///
    /// `model_path`: path to `silero_vad.onnx`.
    pub fn new(model_path: &str) -> Result<Self> {
        let session = ort::Session::builder()
            .context("create ort session builder")?
            .with_intra_threads(1)
            .context("set intra threads")?
            .commit_from_file(model_path)
            .with_context(|| format!("load VAD model from {model_path}"))?;

        debug!("loaded Silero VAD model from {model_path}");

        Ok(Self {
            session,
            h_state: Array3::<f32>::zeros((2, 1, 64)),
            c_state: Array3::<f32>::zeros((2, 1, 64)),
            state: VadInternalState::WaitingForSpeech,
        })
    }

    /// Reset the LSTM state (call between utterances or on error).
    pub fn reset(&mut self) {
        self.h_state = Array3::<f32>::zeros((2, 1, 64));
        self.c_state = Array3::<f32>::zeros((2, 1, 64));
        self.state = VadInternalState::WaitingForSpeech;
    }

    /// Run inference on a single 512-sample frame.
    ///
    /// Returns the speech probability [0.0, 1.0].
    fn infer_frame(&mut self, frame: &[f32]) -> Result<f32> {
        assert_eq!(
            frame.len(),
            VAD_FRAME_SAMPLES,
            "frame must be {VAD_FRAME_SAMPLES} samples"
        );

        let input = ndarray::Array2::from_shape_vec((1, VAD_FRAME_SAMPLES), frame.to_vec())
            .context("shape audio input")?;
        let sr = ndarray::Array1::from_vec(vec![16000i64]);

        let outputs = self
            .session
            .run(ort::inputs![
                "input" => input.view(),
                "sr" => sr.view(),
                "h" => self.h_state.view(),
                "c" => self.c_state.view(),
            ]?)
            .context("VAD inference")?;

        // Extract probability.
        let prob_tensor = outputs["output"]
            .try_extract_tensor::<f32>()
            .context("extract probability")?;
        let prob = prob_tensor.iter().next().copied().unwrap_or(0.0);

        // Update LSTM states.
        if let Ok(hn) = outputs["hn"].try_extract_tensor::<f32>() {
            let data: Vec<f32> = hn.iter().copied().collect();
            if data.len() == 128 {
                self.h_state = Array3::from_shape_vec((2, 1, 64), data)
                    .unwrap_or_else(|_| Array3::zeros((2, 1, 64)));
            }
        }
        if let Ok(cn) = outputs["cn"].try_extract_tensor::<f32>() {
            let data: Vec<f32> = cn.iter().copied().collect();
            if data.len() == 128 {
                self.c_state = Array3::from_shape_vec((2, 1, 64), data)
                    .unwrap_or_else(|_| Array3::zeros((2, 1, 64)));
            }
        }

        trace!("vad prob: {prob:.3}");
        Ok(prob)
    }

    /// Process a 512-sample frame and return a VAD decision.
    ///
    /// Also accumulates audio samples internally — when SpeechEnd is returned,
    /// the full speech segment is included.
    ///
    /// `frame`: 512 f32 samples at 16kHz.
    /// `accumulated_audio`: mutable buffer to accumulate speech audio.
    /// `stream_position`: current position in the audio stream (sample count).
    pub fn process_frame(
        &mut self,
        frame: &[f32],
        accumulated_audio: &mut Vec<f32>,
        stream_position: u64,
    ) -> Result<VadDecision> {
        let prob = self.infer_frame(frame)?;
        let is_speech = prob > SPEECH_THRESHOLD;

        match self.state {
            VadInternalState::WaitingForSpeech => {
                if is_speech {
                    self.state = VadInternalState::InSpeech {
                        speech_frames: 1,
                        silence_frames: 0,
                        start_sample: stream_position,
                    };
                    accumulated_audio.clear();
                    accumulated_audio.extend_from_slice(frame);
                    Ok(VadDecision::SpeechStart)
                } else {
                    Ok(VadDecision::Silence)
                }
            }
            VadInternalState::InSpeech {
                speech_frames,
                silence_frames,
                start_sample,
            } => {
                accumulated_audio.extend_from_slice(frame);

                if is_speech {
                    self.state = VadInternalState::InSpeech {
                        speech_frames: speech_frames + 1,
                        silence_frames: 0,
                        start_sample,
                    };
                    Ok(VadDecision::SpeechContinue)
                } else {
                    let new_silence = silence_frames + 1;

                    if new_silence >= MAX_SILENCE_FRAMES
                        || speech_frames + new_silence >= MAX_SPEECH_FRAMES
                    {
                        // Speech ended.
                        if speech_frames >= MIN_SPEECH_FRAMES {
                            let segment =
                                SpeechSegment::new(accumulated_audio.clone(), start_sample);
                            accumulated_audio.clear();
                            self.state = VadInternalState::WaitingForSpeech;
                            self.reset();
                            Ok(VadDecision::SpeechEnd(segment))
                        } else {
                            // Too short — discard as noise.
                            accumulated_audio.clear();
                            self.state = VadInternalState::WaitingForSpeech;
                            Ok(VadDecision::Silence)
                        }
                    } else {
                        self.state = VadInternalState::InSpeech {
                            speech_frames,
                            silence_frames: new_silence,
                            start_sample,
                        };
                        Ok(VadDecision::SpeechContinue)
                    }
                }
            }
        }
    }

    /// Check if currently in speech.
    pub fn is_in_speech(&self) -> bool {
        matches!(self.state, VadInternalState::InSpeech { .. })
    }
}
