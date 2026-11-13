#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;
use rstest::rstest;

use crate::classifier_message_text::asks::{
    ExtractedAsk, conversation_is_continuing, current_ask_and_system_prompt,
    human_asks_newest_first, is_reminder_only_turn, json_human_asks_newest_first,
    last_human_ask_index, newest_turn_ask, newest_turn_is_human_ask,
};
use crate::classifier_message_text::encrypted::encrypted_classifier_task;
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::image::inline_image_part;
use crate::classifier_message_text::markers::{
    CODEX_REMINDER_MARKERS, DEFAULT_REMINDER_MARKERS, MarkerPair, is_codex_user_agent,
    reminder_markers_for_request,
};
use crate::classifier_message_text::plan::{PLAN_MODE_TOOL_NAME, matched_plan_mode_sentinel};
use crate::classifier_message_text::text::{
    human_text, json_message_text, message_text, strip_reminder_blocks,
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

fn default_markers() -> &'static [MarkerPair<'static>] {
    DEFAULT_REMINDER_MARKERS
}

fn codex_markers() -> &'static [MarkerPair<'static>] {
    CODEX_REMINDER_MARKERS
}

fn one_pair<'marker>(open: &'marker str, close: &'marker str) -> Vec<MarkerPair<'marker>> {
    vec![MarkerPair { open, close }]
}

fn ask_texts(asks: &[crate::classifier_message_text::asks::ExtractedAsk]) -> Vec<&str> {
    asks.iter().map(|ask| ask.text.as_str()).collect()
}

fn resolver(
    resolved: Vec<Vec<Json>>,
) -> impl FnMut(&Json) -> Result<Vec<Json>, ClassifierTextError> {
    let mut remaining = resolved.into_iter();
    move |_item| Ok(remaining.next().unwrap_or_default())
}

mod behaviors;
mod boundaries;
