/// Audio playback via cpal — cross-platform speaker output.
///
/// Plays raw f32 PCM audio through the default output device.
/// Used by TTS to play synthesized speech.
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tracing::{debug, info, warn};

/// Playback handle — queues audio samples and plays them.
pub struct AudioPlayback {
    device: cpal::Device,
    #[allow(dead_code)]
    config: cpal::StreamConfig,
}

impl AudioPlayback {
    /// Create playback handle using the default output device.
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no default output device")?;

        let device_name = device.name().unwrap_or_else(|_| "unknown".into());
        info!("using output device: {device_name}");

        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(22050), // Piper default output rate
            buffer_size: cpal::BufferSize::Default,
        };

        Ok(Self { device, config })
    }

    /// Play audio samples (blocking until complete or stopped).
    ///
    /// `samples`: f32 PCM audio at `sample_rate` Hz.
    /// `sample_rate`: sample rate of the input audio.
    /// `stop_flag`: if set to true externally, playback stops early (for barge-in).
    ///
    /// Returns `true` if playback completed normally, `false` if interrupted.
    pub fn play_blocking(
        &self,
        samples: &[f32],
        sample_rate: u32,
        stop_flag: &Arc<AtomicBool>,
    ) -> Result<bool> {
        if samples.is_empty() {
            return Ok(true);
        }

        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let data = Arc::new(samples.to_vec());
        let position = Arc::new(Mutex::new(0usize));
        let finished = Arc::new(AtomicBool::new(false));

        let data_c = data.clone();
        let pos_c = position.clone();
        let fin_c = finished.clone();
        let stop_c = stop_flag.clone();

        let stream = self
            .device
            .build_output_stream(
                &config,
                move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if stop_c.load(Ordering::Relaxed) {
                        // Stopped — fill with silence.
                        output.fill(0.0);
                        fin_c.store(true, Ordering::Relaxed);
                        return;
                    }

                    let mut pos = pos_c.lock().unwrap();
                    for sample in output.iter_mut() {
                        if *pos < data_c.len() {
                            *sample = data_c[*pos];
                            *pos += 1;
                        } else {
                            *sample = 0.0;
                            fin_c.store(true, Ordering::Relaxed);
                        }
                    }
                },
                |err| {
                    warn!("playback error: {err}");
                },
                None,
            )
            .context("build output stream")?;

        stream.play().context("start playback")?;
        debug!("playing {} samples at {}Hz", samples.len(), sample_rate);

        // Wait for playback to finish or stop_flag.
        loop {
            if finished.load(Ordering::Relaxed) {
                break;
            }
            if stop_flag.load(Ordering::Relaxed) {
                debug!("playback interrupted by stop flag");
                return Ok(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let was_stopped = stop_flag.load(Ordering::Relaxed);
        Ok(!was_stopped)
    }

    /// Play audio samples at the default config sample rate.
    pub fn play_samples(&self, samples: &[f32], sample_rate: u32) -> Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        self.play_blocking(samples, sample_rate, &stop)?;
        Ok(())
    }
}
