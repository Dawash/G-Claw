/// Piper TTS — fast offline neural TTS for English.
///
/// Mirrors: speech.py Piper integration.
/// Piper runs as an external binary (piper.exe / piper) that reads text from stdin
/// and outputs raw 16-bit PCM audio to stdout.
///
/// Cross-platform: Piper has builds for Windows, Linux (x86_64, aarch64), macOS.
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{debug, warn};

use super::{TtsAudio, TtsEngine};

/// Default Piper output sample rate.
const PIPER_SAMPLE_RATE: u32 = 22050;

pub struct PiperTts {
    /// Path to the piper binary.
    binary_path: PathBuf,
    /// Path to the ONNX voice model.
    model_path: PathBuf,
    /// Path to the model's JSON config.
    config_path: PathBuf,
    /// Speaking rate multiplier (1.0 = normal, <1.0 = faster).
    length_scale: f32,
}

impl PiperTts {
    /// Create a new Piper TTS engine.
    ///
    /// `binary_path`: path to piper executable.
    /// `model_path`: path to .onnx voice model (e.g., en_US-lessac-medium.onnx).
    pub fn new(binary_path: &Path, model_path: &Path) -> Result<Self> {
        if !binary_path.exists() {
            bail!("piper binary not found: {}", binary_path.display());
        }
        if !model_path.exists() {
            bail!("piper model not found: {}", model_path.display());
        }

        // Config is typically model_name.onnx.json
        let config_path = model_path.with_extension("onnx.json");

        Ok(Self {
            binary_path: binary_path.to_path_buf(),
            model_path: model_path.to_path_buf(),
            config_path,
            length_scale: 1.0,
        })
    }

    /// Set speaking rate (lower = faster, default 1.0).
    pub fn set_length_scale(&mut self, scale: f32) {
        self.length_scale = scale;
    }

    /// Find piper binary in common locations.
    pub fn find_binary() -> Option<PathBuf> {
        let candidates = if cfg!(windows) {
            vec![
                PathBuf::from("piper/piper.exe"),
                PathBuf::from("../piper/piper.exe"),
                PathBuf::from("C:/tools/piper/piper.exe"),
            ]
        } else {
            vec![
                PathBuf::from("piper/piper"),
                PathBuf::from("../piper/piper"),
                PathBuf::from("/usr/local/bin/piper"),
                PathBuf::from("/usr/bin/piper"),
                // Check PATH
                which("piper"),
            ]
        };

        candidates.into_iter().find(|p| p.exists())
    }
}

/// Try to find a binary in PATH.
fn which(name: &str) -> PathBuf {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    if let Ok(output) = Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(format!("{name}{ext}"))
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return PathBuf::from(path.lines().next().unwrap_or(""));
            }
        }
    }
    PathBuf::from(format!("{name}{ext}"))
}

impl TtsEngine for PiperTts {
    fn synthesize(&self, text: &str) -> Result<TtsAudio> {
        if text.trim().is_empty() {
            return Ok(TtsAudio {
                samples: vec![],
                sample_rate: PIPER_SAMPLE_RATE,
            });
        }

        debug!("piper synthesizing: \"{}\"", &text[..text.len().min(50)]);

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("--model").arg(&self.model_path);
        cmd.arg("--output-raw"); // Raw 16-bit PCM to stdout
        cmd.arg("--length-scale")
            .arg(self.length_scale.to_string());

        if self.config_path.exists() {
            cmd.arg("--config").arg(&self.config_path);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().context("spawn piper process")?;

        // Write text to stdin.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .context("write to piper stdin")?;
            // stdin drops here, closing the pipe
        }

        let output = child.wait_with_output().context("wait for piper")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("piper stderr: {stderr}");
            bail!("piper exited with status {}", output.status);
        }

        // Convert 16-bit PCM to f32 samples.
        let pcm_bytes = &output.stdout;
        let num_samples = pcm_bytes.len() / 2;
        let mut samples = Vec::with_capacity(num_samples);

        for chunk in pcm_bytes.chunks_exact(2) {
            let sample_i16 = i16::from_le_bytes([chunk[0], chunk[1]]);
            samples.push(sample_i16 as f32 / 32768.0);
        }

        debug!("piper produced {} samples ({:.1}s)", samples.len(),
            samples.len() as f32 / PIPER_SAMPLE_RATE as f32);

        Ok(TtsAudio {
            samples,
            sample_rate: PIPER_SAMPLE_RATE,
        })
    }

    fn name(&self) -> &str {
        "piper"
    }

    fn supported_languages(&self) -> &[&str] {
        &["en"]
    }
}
