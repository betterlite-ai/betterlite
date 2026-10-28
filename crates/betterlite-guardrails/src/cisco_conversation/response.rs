use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::tool_argument_parts;
use crate::cisco_conversation::{Fields, InspectMessage, Role, normalize_content};

#[must_use]
pub fn chat_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut messages = Vec::new();
    for choice in Json::field(response, "choices").objects() {
        messages.extend(assistant_message(&chat_choice(&choice)));
    }
    messages
}

#[must_use]
pub fn completion_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut parts = Vec::new();
    for choice in Json::field(response, "choices").objects() {
        push_text(Json::field(&choice, "text"), &mut parts);
    }
    assistant_message(&parts)
}

#[must_use]
pub fn output_messages(response: &Fields) -> Vec<InspectMessage> {
    let mut parts = Vec::new();
    for item in Json::field(response, "output").objects() {
        parts.extend(output_item(&item));
    }
    assistant_message(&parts)
}

fn chat_choice(choice: &Fields) -> Vec<String> {
    let Json::Object(message) = Json::field(choice, "message") else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let content = normalize_content(&Json::field(&message, "content"));
    if !content.is_empty() {
        parts.push(content);
    }
    parts.extend(tool_argument_parts(&message));
    parts.extend(reasoning_parts(&message));
    parts
}

fn output_item(item: &Fields) -> Vec<String> {
    let mut parts = Vec::new();
    for part in Json::field(item, "content").objects() {
        for key in ["text", "reasoning", "thinking"] {
            push_text(Json::field(&part, key), &mut parts);
        }
    }
    for key in ["arguments", "text"] {
        push_text(Json::field(item, key), &mut parts);
    }
    parts
}

#[must_use]
pub fn reasoning_parts(message: &Fields) -> Vec<String> {
    let mut parts = Vec::new();
    push_text(Json::field(message, "reasoning_content"), &mut parts);
    // redacted_thinking data is opaque provider metadata, so only these keys are scanned
    for block in Json::field(message, "thinking_blocks").objects() {
        for key in ["thinking", "reasoning", "text"] {
            push_text(Json::field(&block, key), &mut parts);
        }
    }
    for item in Json::field(message, "reasoning_items").objects() {
        for block in Json::field(&item, "summary").objects() {
            push_text(Json::field(&block, "text"), &mut parts);
        }
        for key in ["text", "reasoning", "reasoning_content"] {
            push_text(Json::field(&item, key), &mut parts);
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
