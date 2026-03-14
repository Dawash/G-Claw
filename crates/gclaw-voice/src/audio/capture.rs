/// Audio capture via cpal — cross-platform microphone input.
///
/// Mirrors: speech.py microphone capture via pyaudio/speech_recognition.
/// Captures 16kHz mono f32 audio into a ring buffer for VAD/STT consumption.
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::{CHANNELS, SAMPLE_RATE};

/// Thread-safe ring buffer for audio samples.
pub struct AudioRingBuffer {
    buffer: Vec<f32>,
    write_pos: usize,
    capacity: usize,
    /// Total samples written (monotonically increasing).
    total_written: u64,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            capacity,
            total_written: 0,
        }
    }

    /// Write samples into the ring buffer.
    pub fn write(&mut self, samples: &[f32]) {
        for &s in samples {
            self.buffer[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % self.capacity;
        }
        self.total_written += samples.len() as u64;
    }

    /// Read the most recent `count` samples. Returns fewer if buffer hasn't filled yet.
    pub fn read_last(&self, count: usize) -> Vec<f32> {
        let available = self.total_written.min(self.capacity as u64) as usize;
        let n = count.min(available);
        let mut out = Vec::with_capacity(n);

        let start = if self.write_pos >= n {
            self.write_pos - n
        } else {
            self.capacity - (n - self.write_pos)
        };

        for i in 0..n {
            let idx = (start + i) % self.capacity;
            out.push(self.buffer[idx]);
        }
        out
    }

    pub fn total_written(&self) -> u64 {
        self.total_written
    }
}

/// Audio capture handle — manages cpal input stream.
pub struct AudioCapture {
    stream: cpal::Stream,
    ring_buffer: Arc<Mutex<AudioRingBuffer>>,
    sample_rate: u32,
}

impl AudioCapture {
    /// Create a new audio capture from the default input device.
    ///
    /// Ring buffer holds `buffer_secs` of audio (default 30s = 480,000 samples @ 16kHz).
    pub fn new(buffer_secs: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device")?;

        let device_name = device.name().unwrap_or_else(|_| "unknown".into());
        info!("using input device: {device_name}");

        let config = cpal::StreamConfig {
            channels: CHANNELS,
            sample_rate: cpal::SampleRate(SAMPLE_RATE),
            buffer_size: cpal::BufferSize::Default,
        };

        let capacity = (SAMPLE_RATE as f32 * buffer_secs) as usize;
        let ring_buffer = Arc::new(Mutex::new(AudioRingBuffer::new(capacity)));
        let rb_clone = ring_buffer.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut rb) = rb_clone.lock() {
                        rb.write(data);
                    }
                },
                |err| {
                    warn!("audio capture error: {err}");
                },
                None, // No timeout
            )
            .context("build input stream")?;

        stream.play().context("start audio capture")?;
        debug!("audio capture started: {SAMPLE_RATE}Hz mono, {buffer_secs}s buffer");

        Ok(Self {
            stream,
            ring_buffer,
            sample_rate: SAMPLE_RATE,
        })
    }

    /// Read the most recent `duration_secs` of audio.
    pub fn read_last(&self, duration_secs: f32) -> Vec<f32> {
        let count = (self.sample_rate as f32 * duration_secs) as usize;
        self.ring_buffer.lock().unwrap().read_last(count)
    }

    /// Read the most recent N samples.
    pub fn read_last_samples(&self, count: usize) -> Vec<f32> {
        self.ring_buffer.lock().unwrap().read_last(count)
    }

    /// Total samples captured since start.
    pub fn total_samples(&self) -> u64 {
        self.ring_buffer.lock().unwrap().total_written()
    }

    /// Pause capture.
    pub fn pause(&self) -> Result<()> {
        self.stream.pause().context("pause audio capture")
    }

    /// Resume capture.
    pub fn resume(&self) -> Result<()> {
        self.stream.play().context("resume audio capture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_basic() {
        let mut rb = AudioRingBuffer::new(10);
        rb.write(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let out = rb.read_last(3);
        assert_eq!(out, vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn ring_buffer_wraparound() {
        let mut rb = AudioRingBuffer::new(4);
        rb.write(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]); // wraps around
        let out = rb.read_last(4);
        assert_eq!(out, vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn ring_buffer_read_more_than_written() {
        let mut rb = AudioRingBuffer::new(10);
        rb.write(&[1.0, 2.0]);
        let out = rb.read_last(5); // only 2 available
        assert_eq!(out, vec![1.0, 2.0]);
    }
}
