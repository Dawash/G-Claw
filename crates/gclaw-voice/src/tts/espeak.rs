/// espeak-ng TTS — multilingual offline fallback.
///
/// Mirrors: speech.py pyttsx3 fallback (which uses espeak-ng on Linux).
/// Cross-platform: available on Windows, Linux, macOS, Android via package managers.
///
/// Runs as external process: `espeak-ng --stdout -v <lang> "<text>"` → raw PCM.
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tracing::{debug, warn};

use super::{TtsAudio, TtsEngine};

/// espeak-ng default output sample rate.
const ESPEAK_SAMPLE_RATE: u32 = 22050;

pub struct EspeakTts {
    /// Path to espeak-ng binary.
    binary_path: PathBuf,
    /// Voice/language (e.g., "en", "hi", "ne").
    voice: String,
    /// Speaking speed in words per minute (default 175).
    speed: u32,
}

impl EspeakTts {
    pub fn new(voice: &str) -> Result<Self> {
        let binary_path = find_espeak().context("espeak-ng not found")?;

        Ok(Self {
            binary_path,
            voice: voice.to_string(),
            speed: 175,
        })
    }

    pub fn set_voice(&mut self, voice: &str) {
        self.voice = voice.to_string();
    }

    pub fn set_speed(&mut self, wpm: u32) {
        self.speed = wpm;
    }
}

fn find_espeak() -> Option<PathBuf> {
    let candidates = if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files\eSpeak NG\espeak-ng.exe"),
            PathBuf::from(r"C:\Program Files (x86)\eSpeak NG\espeak-ng.exe"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/bin/espeak-ng"),
            PathBuf::from("/usr/local/bin/espeak-ng"),
        ]
    };

    candidates.into_iter().find(|p| p.exists()).or_else(|| {
        // Try PATH
        let name = if cfg!(windows) { "espeak-ng.exe" } else { "espeak-ng" };
        Command::new(if cfg!(windows) { "where" } else { "which" })
            .arg(name)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()))
    })
}

impl TtsEngine for EspeakTts {
    fn synthesize(&self, text: &str) -> Result<TtsAudio> {
        if text.trim().is_empty() {
            return Ok(TtsAudio {
                samples: vec![],
                sample_rate: ESPEAK_SAMPLE_RATE,
            });
        }

        debug!("espeak synthesizing: \"{}\"", &text[..text.len().min(50)]);

        let output = Command::new(&self.binary_path)
            .arg("--stdout") // WAV to stdout
            .arg("-v")
            .arg(&self.voice)
            .arg("-s")
            .arg(self.speed.to_string())
            .arg(text)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("run espeak-ng")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("espeak stderr: {stderr}");
            bail!("espeak-ng exited with status {}", output.status);
        }

        // espeak --stdout outputs a WAV file. Skip the 44-byte WAV header.
        let wav_data = &output.stdout;
        if wav_data.len() < 44 {
            bail!("espeak output too short ({} bytes)", wav_data.len());
        }

        // Parse sample rate from WAV header (bytes 24-27, little-endian u32).
        let sample_rate = u32::from_le_bytes([wav_data[24], wav_data[25], wav_data[26], wav_data[27]]);
        let bits_per_sample = u16::from_le_bytes([wav_data[34], wav_data[35]]);

        let pcm_data = &wav_data[44..];
        let samples: Vec<f32> = match bits_per_sample {
            16 => pcm_data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect(),
            8 => pcm_data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
            _ => bail!("unsupported bits per sample: {bits_per_sample}"),
        };

        debug!("espeak produced {} samples at {}Hz ({:.1}s)",
            samples.len(), sample_rate,
            samples.len() as f32 / sample_rate as f32);

        Ok(TtsAudio {
            samples,
            sample_rate,
        })
    }

    fn name(&self) -> &str {
        "espeak-ng"
    }

    fn supported_languages(&self) -> &[&str] {
        // espeak-ng supports 100+ languages
        &["en", "hi", "ne", "es", "fr", "de", "zh", "ja", "ko", "ru", "ar", "pt"]
    }
}
