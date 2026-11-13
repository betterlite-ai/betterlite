use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::tool_argument_parts;
use crate::cisco_conversation::{Fields, InspectMessage, Role, normalize_content};

#[must_use]
pub fn chat_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut messages = Vec::new();
    for choice in Json::parse(response.get("choices").map_or("null", |raw| raw.get())).objects() {
        messages.extend(assistant_message(&chat_choice(&choice)));
    }
    messages
}

#[must_use]
pub fn completion_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut parts = Vec::new();
    for choice in Json::parse(response.get("choices").map_or("null", |raw| raw.get())).objects() {
        push_text(
            Json::parse(choice.get("text").map_or("null", |raw| raw.get())),
            &mut parts,
        );
    }
    assistant_message(&parts)
}

#[must_use]
pub fn output_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut parts = Vec::new();
    for item in Json::parse(response.get("output").map_or("null", |raw| raw.get())).objects() {
        parts.extend(output_item(&item));
    }
    assistant_message(&parts)
}

fn chat_choice(choice: &Fields) -> Vec<String> {
    let Json::Object(message) = Json::parse(choice.get("message").map_or("null", |raw| raw.get()))
    else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let content = normalize_content(&Json::parse(
        message.get("content").map_or("null", |raw| raw.get()),
    ));
    if !content.is_empty() {
        parts.push(content);
    }
    parts.extend(tool_argument_parts(&message));
    parts.extend(reasoning_parts(&message));
    parts
}

fn output_item(item: &Fields) -> Vec<String> {
    let mut parts = Vec::new();
    for part in Json::parse(item.get("content").map_or("null", |raw| raw.get())).objects() {
        for key in ["text", "reasoning", "thinking"] {
            push_text(
                Json::parse(part.get(key).map_or("null", |raw| raw.get())),
                &mut parts,
            );
        }
    }
    for key in ["arguments", "text"] {
        push_text(
            Json::parse(item.get(key).map_or("null", |raw| raw.get())),
            &mut parts,
        );
    }
    parts
}

#[must_use]
pub fn reasoning_parts(message: &Fields) -> Vec<String> {
    let mut parts = Vec::new();
    let reasoning = message
        .get("reasoning_content")
        .map_or("null", |raw| raw.get());
    push_text(Json::parse(reasoning), &mut parts);
    // redacted_thinking data is opaque provider metadata, so only these keys are scanned
    let thinking_items = message
        .get("thinking_blocks")
        .map_or("null", |raw| raw.get());
    for block in Json::parse(thinking_items).objects() {
        for key in ["thinking", "reasoning", "text"] {
            push_text(
                Json::parse(block.get(key).map_or("null", |raw| raw.get())),
                &mut parts,
            );
        }
    }
    let reasoning_items = message
        .get("reasoning_items")
        .map_or("null", |raw| raw.get());
    for item in Json::parse(reasoning_items).objects() {
        for block in Json::parse(item.get("summary").map_or("null", |raw| raw.get())).objects() {
            push_text(
                Json::parse(block.get("text").map_or("null", |raw| raw.get())),
                &mut parts,
            );
        }
        for key in ["text", "reasoning", "reasoning_content"] {
            push_text(
                Json::parse(item.get(key).map_or("null", |raw| raw.get())),
                &mut parts,
            );
        }
    }
    parts
}

fn push_text(value: Json, parts: &mut Vec<String>) {
    if let Json::Text(text) = value
        && !text.is_empty()
    {
        parts.push(text);
    }
}

fn assistant_message(parts: &[String]) -> Vec<InspectMessage> {
    if parts.is_empty() {
        Vec::new()
    } else {
        vec![InspectMessage {
            role: Role::Assistant,
            content: parts.join(" "),
        }]
    }
}
