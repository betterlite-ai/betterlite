#![expect(
    clippy::disallowed_types,
    reason = "OpenCode leaves only tool state input and output as provider-defined JSON"
)]

use serde::Deserialize;
use serde_json::Value;

use crate::types::JsonObject;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Line {
    #[serde(rename = "step_start")]
    StepStart {
        #[serde(rename = "sessionID")]
        session_id: Option<String>,
    },
    #[serde(rename = "text")]
    Text {
        #[serde(rename = "sessionID")]
        session_id: Option<String>,
        part: TextPart,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(rename = "sessionID")]
        session_id: Option<String>,
        part: TextPart,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(rename = "sessionID")]
        session_id: Option<String>,
        part: ToolPart,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(rename = "sessionID")]
        session_id: Option<String>,
        error: ErrorEvent,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TextPart {
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolPart {
    #[serde(default)]
    pub(crate) tool: Option<String>,
    #[serde(rename = "callID", default)]
    pub(crate) call_id: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) state: ToolState,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ToolState {
    #[serde(default)]
    pub(crate) input: Option<Value>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) output: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ErrorEvent {
    Text(String),
    Object(ErrorDetails),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorDetails {
    #[serde(default)]
    data: Option<ErrorData>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorData {
    #[serde(default)]
    pub(crate) message: Option<String>,
}

pub(crate) fn error_message(error: &ErrorEvent) -> String {
    match error {
        ErrorEvent::Text(message) if !message.is_empty() => message.clone(),
        ErrorEvent::Text(_) => "opencode reported an error".to_owned(),
        ErrorEvent::Object(details) => details
            .data
            .as_ref()
            .and_then(|value| value.message.clone())
            .or_else(|| details.name.clone())
            .unwrap_or_else(|| "opencode reported an error".to_owned()),
    }
}

impl Line {
    pub(crate) fn session_id(&self) -> Option<&str> {
        match self {
            Self::StepStart { session_id }
            | Self::Text { session_id, .. }
            | Self::Reasoning { session_id, .. }
            | Self::ToolUse { session_id, .. }
            | Self::Error { session_id, .. } => session_id.as_deref(),
            Self::Other => None,
        }
    }
}

pub(crate) fn parse(line: &JsonObject) -> Line {
    let value = Value::Object(line.clone().into_iter().collect());
    serde_json::from_value(value).unwrap_or(Line::Other)
}
