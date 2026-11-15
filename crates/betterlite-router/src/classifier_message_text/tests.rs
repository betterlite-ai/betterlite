#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;

use crate::classifier_message_text::asks::{ExtractedAsk, human_asks_newest_first};
use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::{
    CODEX_REMINDER_MARKERS, DEFAULT_REMINDER_MARKERS, MarkerPair,
};
use crate::classifier_message_text::plan::matched_plan_mode_sentinel;
use crate::classifier_message_text::text::strip_reminder_blocks;

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

fn json_text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn json_number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn json_object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn kind_part(kind: &str, key: &str, value: Json) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: IndexMap::from([(key.to_owned(), serde_json::to_value(value).unwrap())]),
    }
}

fn text_part(value: &str) -> ContentPart {
    kind_part("text", "text", json_text(value))
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

fn assert_strip(text: &str, expected: &str) {
    assert_eq!(strip_reminder_blocks(text, default_markers()), expected);
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

fn encrypted_part(value: Json) -> Json {
    json_object(&[
        ("type", json_text("encrypted_content")),
        ("encrypted_content", value),
    ])
}

fn typed_item(kind: &str, content: Vec<Json>) -> Json {
    json_object(&[("type", json_text(kind)), ("content", Json::List(content))])
}

fn tools_body(tools: Vec<Json>) -> Json {
    json_object(&[("tools", Json::List(tools))])
}

fn system_body(system: Json) -> Json {
    json_object(&[("system", system)])
}

fn plan_body_result(body: &Json) -> Option<String> {
    matched_plan_mode_sentinel(Some(body), None, &[], default_markers()).unwrap()
}

fn encrypted_item() -> Json {
    json_object(&[
        ("type", json_text("agent_message")),
        ("id", json_text("item-1")),
        (
            "content",
            Json::List(vec![
                json_object(&[
                    ("type", json_text("input_text")),
                    ("text", json_text("hello")),
                ]),
                encrypted_part(json_text("blob")),
                json_object(&[
                    ("type", json_text("tool_result")),
                    ("content", json_text("dropped")),
                ]),
            ]),
        ),
    ])
}

fn resolved_ask() -> Vec<Json> {
    vec![json_object(&[
        ("role", json_text("user")),
        (
            "content",
            Json::List(vec![json_object(&[
                ("type", json_text("text")),
                ("text", json_text("hello blob")),
            ])]),
        ),
    ])]
}

mod behaviors;
mod boundaries;
