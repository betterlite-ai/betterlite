use serde::Deserialize;

use crate::types::JsonObject;

#[expect(
    clippy::disallowed_types,
    reason = "OpenCode tool input and output are provider-defined JSON"
)]
type ProviderJson = serde_json::Value;

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
        #[serde(default)]
        error: Option<ErrorEvent>,
    },
    Other,
    Malformed {
        session_id: Option<String>,
        line: String,
    },
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
    pub(crate) input: Option<ProviderJson>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) output: Option<ProviderJson>,
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
struct ErrorData {
    #[serde(default)]
    message: Option<String>,
}

pub(crate) fn error_message(error: &ErrorEvent) -> String {
    if let ErrorEvent::Text(message) = error {
        return if message.is_empty() {
            "opencode reported an error".to_owned()
        } else {
            message.clone()
        };
    }
    match error {
        ErrorEvent::Text(_) => "opencode reported an error".to_owned(),
        ErrorEvent::Object(details) => details
            .data
            .as_ref()
            .and_then(|data| {
                data.message
                    .as_deref()
                    .filter(|message| !message.is_empty())
            })
            .map(str::to_owned)
            .or_else(|| {
                details
                    .name
                    .as_ref()
                    .filter(|name| !name.is_empty())
                    .cloned()
            })
            .unwrap_or_else(|| "opencode reported an error".to_owned()),
    }
}

pub(crate) fn value_to_string(value: &ProviderJson) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    value.to_string()
}

pub(crate) fn is_truthy(value: &ProviderJson) -> bool {
    match value {
        ProviderJson::Null => false,
        ProviderJson::Bool(boolean) => *boolean,
        ProviderJson::Number(number) => number != &serde_json::Number::from(0_u8),
        ProviderJson::String(text) => !text.is_empty(),
        ProviderJson::Array(array) => !array.is_empty(),
        ProviderJson::Object(object) => !object.is_empty(),
    }
}

impl Line {
    pub(crate) fn session_id(&self) -> Option<&str> {
        match self {
            Self::StepStart { session_id }
            | Self::Text { session_id, .. }
            | Self::Reasoning { session_id, .. }
            | Self::ToolUse { session_id, .. }
            | Self::Error { session_id, .. }
            | Self::Malformed { session_id, .. } => session_id.as_deref(),
            Self::Other => None,
        }
    }
}

pub(crate) fn parse(line: &JsonObject) -> Line {
    let line_value = ProviderJson::Object(line.clone().into_iter().collect());
    let event_type = line.get("type").and_then(|value| value.as_str());
    match serde_json::from_value(line_value) {
        Ok(parsed) => parsed,
        Err(error)
            if event_type.is_some_and(|kind| {
                ["step_start", "text", "reasoning", "tool_use", "error"].contains(&kind)
            }) =>
        {
            Line::Malformed {
                session_id: line
                    .get("sessionID")
                    .filter(|session| is_truthy(session))
                    .map(value_to_string),
                line: error.to_string(),
            }
        }
        Err(_) => Line::Other,
    }
}
