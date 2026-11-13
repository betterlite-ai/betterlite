use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::primitives::json_value::JsonValue;

mod tests;

const OUTPUT_ITEM_TYPES: [&str; 2] = ["function_call_output", "custom_tool_call_output"];

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptData {
    pub messages: Vec<PromptMessage>,
    pub input: Option<PromptInput>,
    pub metadata: Option<Metadata>,
    pub litellm_metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptMessage {
    pub content: Option<MessageContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPiece>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ContentPiece {
    Text(String),
    Part(ContentPart),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PromptInput {
    Text(String),
    Items(Vec<InputItem>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InputItem {
    Text(String),
    Object(InputObject),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct InputObject {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: Option<String>,
    pub content: Option<MessageContent>,
    pub summary: Option<MessageContent>,
    pub output: Option<MessageContent>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_guardrails: Option<Vec<String>>,
    #[serde(flatten)]
    pub other: BTreeMap<String, JsonValue>,
}

#[must_use]
pub fn iter_message_text(data: &PromptData) -> Vec<String> {
    let mut texts = Vec::new();
    for message in &data.messages {
        push_content_text(message.content.as_ref(), &mut texts);
    }
    if let Some(input) = &data.input {
        push_input_text(input, &mut texts);
    }
    texts
}

fn push_input_text(input: &PromptInput, texts: &mut Vec<String>) {
    match input {
        PromptInput::Text(text) => push_if_present(text, texts),
        PromptInput::Items(items) => {
            for item in items {
                push_input_item(item, texts);
            }
        }
    }
}

fn push_input_item(item: &InputItem, texts: &mut Vec<String>) {
    match item {
        InputItem::Text(text) => push_if_present(text, texts),
        InputItem::Object(object) => push_input_object(object, texts),
    }
}

fn push_input_object(object: &InputObject, texts: &mut Vec<String>) {
    if let Some(text) = object.text.as_deref().filter(|text| !text.is_empty()) {
        texts.push(text.to_owned());
        return;
    }
    if object.kind.as_deref() == Some("reasoning") {
        push_content_text(object.content.as_ref(), texts);
        push_content_text(object.summary.as_ref(), texts);
        return;
    }
    if object.content.is_some() {
        push_content_text(object.content.as_ref(), texts);
        return;
    }
    let is_output_item = object
        .kind
        .as_deref()
        .is_some_and(|kind| OUTPUT_ITEM_TYPES.contains(&kind));
    if !is_output_item {
        return;
    }
    if let Some(output) = &object.output {
        push_content_text(Some(output), texts);
    }
}

fn push_content_text(text_content: Option<&MessageContent>, texts: &mut Vec<String>) {
    match text_content {
        None => {}
        Some(MessageContent::Text(text)) => push_if_present(text, texts),
        Some(MessageContent::Parts(pieces)) => {
            for piece in pieces {
                push_piece_text(piece, texts);
            }
        }
    }
}

fn push_piece_text(piece: &ContentPiece, texts: &mut Vec<String>) {
    match piece {
        ContentPiece::Text(text) => push_if_present(text, texts),
        ContentPiece::Part(part) => push_required_text(part.text.as_deref(), texts),
    }
}

fn push_required_text(raw: Option<&str>, texts: &mut Vec<String>) {
    if let Some(text) = raw.filter(|text| !text.is_empty()) {
        texts.push(text.to_owned());
    }
}

fn push_if_present(text: &str, texts: &mut Vec<String>) {
    if !text.is_empty() {
        texts.push(text.to_owned());
    }
}
