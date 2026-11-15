#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;

use crate::classifier_message_text::asks::{ExtractedAsk, human_asks_newest_first};
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::{
    CODEX_REMINDER_MARKERS, DEFAULT_REMINDER_MARKERS, MarkerPair,
};

const SYSTEM_SENTINEL: &str = "You are currently running in \"Plan\" mode.";
const TAIL_SENTINEL: &str = "Plan mode is active";
const STILL_ACTIVE: &str = "Plan mode still active";

fn chat(role: Role, content: Option<MessageContent>) -> ChatMessage {
    ChatMessage {
        role,
        content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn text(role: Role, value: &str) -> ChatMessage {
    chat(role, Some(MessageContent::Text(value.to_owned())))
}

fn parts(role: Role, parts: Vec<ContentPart>) -> ChatMessage {
    chat(role, Some(MessageContent::Parts(parts)))
}

fn kind_part<T: serde::Serialize>(kind: &str, key: &str, value: T) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: IndexMap::from([(key.to_owned(), serde_json::to_value(value).unwrap())]),
    }
}

fn text_part(value: &str) -> ContentPart {
    kind_part("text", "text", value)
}

fn body<T: serde::Serialize>(value: T) -> Json {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

const fn default_markers() -> &'static [MarkerPair<'static>] {
    DEFAULT_REMINDER_MARKERS
}

const fn codex_markers() -> &'static [MarkerPair<'static>] {
    CODEX_REMINDER_MARKERS
}

fn one_pair<'marker>(open: &'marker str, close: &'marker str) -> Vec<MarkerPair<'marker>> {
    vec![MarkerPair { open, close }]
}

fn ask_texts(asks: &[ExtractedAsk]) -> Vec<&str> {
    asks.iter().map(|ask| ask.text.as_str()).collect()
}

fn last_ask_index(messages: &[ChatMessage], markers: &[MarkerPair<'_>]) -> Option<usize> {
    human_asks_newest_first(messages, markers)
        .unwrap()
        .first()
        .map(|ask| ask.message_index)
}

fn resolver(
    resolved: Vec<Vec<Json>>,
) -> impl FnMut(&Json) -> Result<Vec<Json>, ClassifierTextError> {
    let mut remaining = resolved.into_iter();
    move |_item| Ok(remaining.next().unwrap_or_default())
}

fn encrypted_item() -> Json {
    body(serde_json::json!({
        "type": "agent_message",
        "id": "item-1",
        "content": [
            {"type": "input_text", "text": "hello"},
            {"type": "encrypted_content", "encrypted_content": "blob"},
            {"type": "tool_result", "content": "dropped"}
        ]
    }))
}

fn resolved_ask() -> Vec<Json> {
    vec![body(
        serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hello blob"}]}),
    )]
}

mod behaviors;
mod boundaries;
