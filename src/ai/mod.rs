pub mod discovery;
pub mod harness;
pub mod runner;

use crate::runner::ModelSchemaInfo;
use serde::{Deserialize, Serialize};

/// Built-in CLI AI agents supported by Laralah.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CliProviderId {
    Codex,
    ClaudeCode,
    Agy,
    Qoder,
    Opencode,
}

impl CliProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            CliProviderId::Codex => "codex",
            CliProviderId::ClaudeCode => "claude_code",
            CliProviderId::Agy => "agy",
            CliProviderId::Qoder => "qoder",
            CliProviderId::Opencode => "opencode",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CliProviderId::Codex => "Codex",
            CliProviderId::ClaudeCode => "Claude Code",
            CliProviderId::Agy => "Antigravity (Agy)",
            CliProviderId::Qoder => "Qoder",
            CliProviderId::Opencode => "OpenCode",
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            CliProviderId::Codex => "Codex",
            CliProviderId::ClaudeCode => "Claude",
            CliProviderId::Agy => "Agy",
            CliProviderId::Qoder => "Qoder",
            CliProviderId::Opencode => "OpenCode",
        }
    }

    pub fn icon_path(&self) -> &'static str {
        match self {
            CliProviderId::Codex => "icons/agent-codex.svg",
            CliProviderId::ClaudeCode => "icons/agent-claude.svg",
            CliProviderId::Agy => "icons/agent-agy.svg",
            CliProviderId::Qoder => "icons/agent-qoder.svg",
            CliProviderId::Opencode => "icons/agent-opencode.svg",
        }
    }

    pub fn binaries(&self) -> &'static [&'static str] {
        match self {
            CliProviderId::Codex => &["codex"],
            CliProviderId::ClaudeCode => &["claude"],
            CliProviderId::Agy => &["agy"],
            CliProviderId::Qoder => &["qodercli", "qoder"],
            CliProviderId::Opencode => &["opencode"],
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "codex" => Some(CliProviderId::Codex),
            "claude_code" | "claude" => Some(CliProviderId::ClaudeCode),
            "agy" | "antigravity" => Some(CliProviderId::Agy),
            "qoder" | "qodercli" => Some(CliProviderId::Qoder),
            "opencode" => Some(CliProviderId::Opencode),
            _ => None,
        }
    }

    pub fn all() -> [CliProviderId; 5] {
        [
            CliProviderId::Codex,
            CliProviderId::ClaudeCode,
            CliProviderId::Agy,
            CliProviderId::Qoder,
            CliProviderId::Opencode,
        ]
    }
}

#[derive(Debug, Clone)]
pub struct CliAgentRuntimeState {
    pub available: bool,
    pub connected: Option<bool>,
    pub checking: bool,
    pub loading_models: bool,
    pub version: Option<String>,
    pub check_message: Option<String>,
    pub model_list: Option<AiModelList>,
    pub model_error: Option<String>,
}

impl CliAgentRuntimeState {
    pub fn new(available: bool) -> Self {
        Self {
            available,
            connected: None,
            checking: false,
            loading_models: false,
            version: None,
            check_message: None,
            model_list: None,
            model_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiModelList {
    pub provider: CliProviderId,
    pub source: String,
    pub models: Vec<AiModel>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct AiToolStep {
    pub id: String,
    pub summary: String,
    pub done: bool,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiMessageStatus {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct AiChatMessage {
    pub id: u64,
    pub user_prompt: String,
    pub agent: CliProviderId,
    pub model_label: String,
    pub effort: Option<String>,
    pub tool_steps: Vec<AiToolStep>,
    pub streaming_text: String,
    pub explanation: String,
    pub php_code: Option<String>,
    pub written_to_editor: bool,
    pub lint_passed: bool,
    pub status: AiMessageStatus,
}

#[derive(Debug, Clone)]
pub struct AiChatSession {
    pub id: u64,
    pub title: String,
    pub messages: Vec<AiChatMessage>,
}

impl AiChatSession {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            title: format!("Session {id}"),
            messages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AiUiEvent {
    ProvidersDetected {
        statuses: Vec<(CliProviderId, bool)>,
    },
    CliChecked {
        provider: CliProviderId,
        available: bool,
        connected: bool,
        version: Option<String>,
        message: String,
    },
    ModelsLoaded {
        provider: CliProviderId,
        result: Result<AiModelList, String>,
    },
    TurnToolStep {
        msg_id: u64,
        step: AiToolStep,
    },
    TurnStreamDelta {
        msg_id: u64,
        full_text: String,
    },
    TurnResetStream {
        msg_id: u64,
    },
    TurnCompleted {
        msg_id: u64,
        explanation: String,
        php_code: Option<String>,
        lint_passed: bool,
        project_key: String,
        cached_models: Vec<ModelSchemaInfo>,
        cached_tables: Option<Vec<String>>,
    },
    TurnFailed {
        msg_id: u64,
        error: String,
    },
    TurnCancelled {
        msg_id: u64,
    },
}

pub fn format_effort_label(token: &str) -> String {
    match token.trim().to_ascii_lowercase().as_str() {
        "" | "default" => "Default".to_string(),
        "none" => "None".to_string(),
        "minimal" => "Minimal".to_string(),
        "low" => "Low".to_string(),
        "medium" => "Medium".to_string(),
        "high" => "High".to_string(),
        "xhigh" => "Extra High".to_string(),
        "max" => "Max".to_string(),
        "ultra" => "Ultra".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => "Default".to_string(),
            }
        }
    }
}
