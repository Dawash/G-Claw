/// gclaw-voice binary — native voice shell for G-Claw.
///
/// Usage:
///   gclaw-voice                          # Use default config
///   gclaw-voice --config path/config.json
///   gclaw-voice --port 19820             # IPC port override
///   gclaw-voice --vad-model path/silero_vad.onnx
///   gclaw-voice --whisper-model path/ggml-base.bin
///   gclaw-voice --piper-bin path/piper --piper-model path/voice.onnx
use anyhow::{Context, Result};
use tracing::info;

use gclaw_voice::shell::{VoiceShell, VoiceShellConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("gclaw-voice v{}", env!("CARGO_PKG_VERSION"));

    // Parse args (simple, no clap dependency).
    let args: Vec<String> = std::env::args().collect();
    let mut config = VoiceShellConfig::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                if i < args.len() {
                    // Load from config.json and apply.
                    let cfg = gclaw_config::load_config(Some(args[i].as_ref()))
                        .context("load config")?;
                    config.ai_name = cfg.ai_name;
                    if cfg.language != "auto" {
                        config.language = Some(cfg.language);
                    }
                }
            }
            "--port" | "-p" => {
                i += 1;
                if i < args.len() {
                    config.ipc_port = args[i].parse().context("parse port")?;
                }
            }
            "--vad-model" => {
                i += 1;
                if i < args.len() {
                    config.vad_model_path = args[i].clone();
                }
            }
            "--whisper-model" => {
                i += 1;
                if i < args.len() {
                    config.whisper_model_path = args[i].clone();
                }
            }
            "--piper-bin" => {
                i += 1;
                if i < args.len() {
                    config.piper_binary_path = Some(args[i].clone());
                }
            }
            "--piper-model" => {
                i += 1;
                if i < args.len() {
                    config.piper_model_path = Some(args[i].clone());
                }
            }
            "--ai-name" => {
                i += 1;
                if i < args.len() {
                    config.ai_name = args[i].clone();
                }
            }
            "--help" | "-h" => {
                println!("gclaw-voice — Native voice shell for G-Claw");
                println!();
                println!("Options:");
                println!("  --config, -c <path>       Path to config.json");
                println!("  --port, -p <port>         IPC port (default: 19820)");
                println!("  --vad-model <path>        Path to silero_vad.onnx");
                println!("  --whisper-model <path>    Path to ggml-*.bin");
                println!("  --piper-bin <path>        Path to piper binary");
                println!("  --piper-model <path>      Path to piper voice .onnx");
                println!("  --ai-name <name>          AI name for wake word (default: G)");
                println!("  --help, -h                Show this help");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let shell = VoiceShell::new(config);
    shell.run().await
}
