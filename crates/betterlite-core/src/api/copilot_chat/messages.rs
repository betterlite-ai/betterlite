use betterlite_config::sections::copilot_chat::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// a chat message as sent: only the role is read, every other field passes through untouched
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CopilotMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Json>,
}

// LiteLLM copies each system message to role assistant unless the setting turns the conversion off
#[must_use]
pub fn transform_messages(
    mut messages: Vec<CopilotMessage>,
    settings: &Settings,
) -> Vec<CopilotMessage> {
    if settings.disable_system_to_assistant {
        return messages;
    }
    for message in &mut messages {
        if message.role.as_deref() == Some("system") {
            message.role = Some(String::from("assistant"));
        }
    }
    messages
}

#[must_use]
pub fn initiator(messages: &[CopilotMessage]) -> &'static str {
    if messages
        .iter()
        .any(|message| matches!(message.role.as_deref(), Some("tool" | "assistant")))
    {
        "agent"
    } else {
        "user"
    }
}

// a list part counts as vision content by its image_url key or by type image_url, whichever it has
#[must_use]
pub fn has_vision_content(messages: &[CopilotMessage]) -> bool {
    messages.iter().any(|message| {
        let Some(Json::List(parts)) = message.fields.get("content") else {
            return false;
        };
        parts.iter().any(|part| {
            matches!(part, Json::Object(fields)
                if fields.contains_key("image_url")
                    || matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "image_url"))
        })
    })
}
