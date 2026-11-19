use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent};

use super::error::PlacementError;

// the prompt carriers the resolved message list never holds: instructions, the top-level system block, tools
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutOfBand<'request> {
    pub instructions: Option<&'request Json>,
    pub system: Option<&'request Json>,
    pub body_tools: Option<&'request Json>,
    pub request_tools: Option<&'request Json>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestFacts<'request> {
    pub messages: Option<&'request [ChatMessage]>,
    pub out_of_band: OutOfBand<'request>,
    pub compaction_pending: bool,
}

// _counted_request_tokens: the tokenizer's count of the messages plus the out-of-band text, as a failure when it cannot count
#[async_trait]
pub trait RequestCounter: Send + Sync {
    async fn count(&self, messages: &[ChatMessage], text: &str) -> Result<u64, PlacementError>;
}

// _out_of_band_request_text: instructions only as text, the system block as text or JSON, then the tools
#[must_use]
pub fn out_of_band_text(out_of_band: &OutOfBand<'_>) -> String {
    let mut text = String::new();
    if let Some(Json::Text(instructions)) = out_of_band.instructions {
        text.push_str(instructions);
    }
    if let Some(system) = out_of_band.system {
        match system {
            Json::Null => {}
            Json::Text(value) => text.push_str(value),
            other @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                text.push_str(&serde_json::to_string(other).unwrap_or_default());
            }
        }
    }
    if let Some(tools) = selected_tools(out_of_band) {
        text.push_str(&serde_json::to_string(tools).unwrap_or_default());
    }
    text
}

// _request_byte_upper_bound: UTF-8 bytes of every message's content plus the out-of-band text
#[must_use]
pub fn request_byte_upper_bound(
    messages: Option<&[ChatMessage]>,
    out_of_band: &OutOfBand<'_>,
) -> u64 {
    let content: usize = messages.unwrap_or_default().iter().map(content_bytes).sum();
    let bytes = content.saturating_add(out_of_band_text(out_of_band).len());
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

// str(message.get("content") or "") in UTF-8 bytes; parts measure as their compact JSON
fn content_bytes(message: &ChatMessage) -> usize {
    match &message.content {
        None => 0,
        Some(MessageContent::Text(text)) => text.len(),
        Some(MessageContent::Parts(parts)) if parts.is_empty() => 0,
        Some(content) => serde_json::to_string(content).unwrap_or_default().len(),
    }
}

// the body's declared tools list is measured as given; only an absent or null body asks the request's list
fn selected_tools<'request>(out_of_band: &OutOfBand<'request>) -> Option<&'request Json> {
    let declared = match out_of_band.body_tools {
        Some(Json::Null) | None => out_of_band.request_tools,
        Some(tools) => Some(tools),
    };
    declared.filter(|tools| !matches!(*tools, Json::Null))
}
