/// IPC message types shared between Rust voice shell, Go tool runtime, and Python brain.
///
/// Mirrors the message flow from the architecture plan:
///   Voice Shell → Brain: UserSpeech, WakeWordDetected, BargeIn, VoiceCommand, Ready
///   Brain → Voice Shell: Speak, SpeakInterruptible, StopSpeaking, SetMicState, Configure, Shutdown
///   Brain → Tool Runtime: ToolExecute
///   Tool Runtime → Brain: ToolResult
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Mic state (mirrors core/state.py AudioState.mic_state)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MicState {
    Idle,
    Listening,
    Processing,
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

// ---------------------------------------------------------------------------
// Session mode (mirrors core/state.py SessionState.mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionMode {
    Idle,
    Active,
}

// ---------------------------------------------------------------------------
// Voice Shell → Brain messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSpeech {
    pub text: String,
    pub language: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BargeIn {
    pub text: String,
}

/// Meta voice commands: skip, shorter, repeat, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceCommand {
    pub command: String,
}

// ---------------------------------------------------------------------------
// Brain → Voice Shell messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigureVoice {
    pub stt_engine: Option<String>,
    pub language: Option<String>,
    pub ai_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMicStateRequest {
    pub state: MicState,
}

// ---------------------------------------------------------------------------
// Brain → Tool Runtime messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecute {
    pub tool: String,
    pub args: serde_json::Value,
    pub user_input: String,
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Tool Runtime → Brain messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
    pub cache_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Unified envelope — every IPC message is wrapped in this
// ---------------------------------------------------------------------------

/// Unified envelope — every IPC message is wrapped in this.
///
/// Uses externally tagged representation for maximum msgpack compatibility.
/// Unit variants serialize as `"Ping"`, tuple variants as `{"UserSpeech": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // Voice → Brain
    UserSpeech(UserSpeech),
    WakeWordDetected,
    BargeIn(BargeIn),
    VoiceCommand(VoiceCommand),
    Ready,

    // Brain → Voice
    Speak(SpeakRequest),
    SpeakInterruptible(SpeakRequest),
    StopSpeaking,
    SetMicState(SetMicStateRequest),
    Configure(ConfigureVoice),
    Shutdown,

    // Brain → Tools
    ToolExecute(ToolExecute),

    // Tools → Brain
    ToolResult(ToolResult),

    // Heartbeat (bidirectional)
    Ping,
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_user_speech() {
        let msg = Message::UserSpeech(UserSpeech {
            text: "hello world".into(),
            language: "en".into(),
            confidence: 0.95,
        });
        let bytes = rmp_serde::to_vec(&msg).unwrap();
        let decoded: Message = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            Message::UserSpeech(s) => {
                assert_eq!(s.text, "hello world");
                assert_eq!(s.language, "en");
                assert!((s.confidence - 0.95).abs() < 0.001);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_wake_word() {
        let msg = Message::WakeWordDetected;
        let bytes = rmp_serde::to_vec(&msg).unwrap();
        let decoded: Message = rmp_serde::from_slice(&bytes).unwrap();
        assert!(matches!(decoded, Message::WakeWordDetected));
    }

    #[test]
    fn roundtrip_tool_execute() {
        let msg = Message::ToolExecute(ToolExecute {
            tool: "get_weather".into(),
            args: serde_json::json!({"city": "London"}),
            user_input: "what's the weather in London".into(),
            mode: "quick".into(),
        });
        let bytes = rmp_serde::to_vec(&msg).unwrap();
        let decoded: Message = rmp_serde::from_slice(&bytes).unwrap();
        match decoded {
            Message::ToolExecute(t) => {
                assert_eq!(t.tool, "get_weather");
                assert_eq!(t.args["city"], "London");
            }
            _ => panic!("wrong variant"),
        }
    }
}
