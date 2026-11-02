use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::response::reasoning_parts;
use crate::cisco_conversation::{Fields, normalize_content};
use crate::cisco_redaction::chat_output::list_field::{
    ListField, map_items, redact_field_when_present, redact_list_field,
};
use crate::cisco_redaction::{clear_tool_call_arguments, set_serializable_field};

pub mod list_field;

#[must_use]
pub fn redact_chat_output(
    response: Option<&mut Fields>,
    sanitized_text: Option<&str>,
    sanitized_messages: Option<&[Fields]>,
) -> bool {
    let Some(present_response) = response else {
        return false;
    };
    if matches!(
        Json::field(present_response, "object"),
        Json::Text(object) if object == "text_completion"
    ) {
        return redact_list_field(
            Some(present_response),
            sanitized_text,
            sanitized_messages,
            ListField::TextCompletion,
        );
    }
    if matches!(Json::field(present_response, "choices"), Json::List(_)) {
        return redact_model_response(Some(present_response), sanitized_text, sanitized_messages);
    }
    if matches!(Json::field(present_response, "output"), Json::List(_)) {
        return redact_list_field(
            Some(present_response),
            sanitized_text,
            sanitized_messages,
            ListField::ResponsesOutput,
        );
    }
    false
}

#[must_use]
pub fn redact_model_response(
    response: Option<&mut Fields>,
    sanitized_text: Option<&str>,
    sanitized_messages: Option<&[Fields]>,
) -> bool {
    let Some(present_response) = response else {
        return false;
    };
    let Json::List(raw_choices) = Json::field(present_response, "choices") else {
        return false;
    };
    if let Some(messages) = sanitized_messages.filter(|list| !list.is_empty()) {
        let (choices, applied) =
            redact_choices_with_messages(&raw_choices, sanitized_text, messages);
        set_serializable_field(present_response, "choices", &choices);
        return applied;
    }
    let Some(text) = sanitized_text.filter(|text| !text.is_empty()) else {
        return false;
    };
    let (choices, applied) = redact_choices_with_text(&raw_choices, text);
    set_serializable_field(present_response, "choices", &choices);
    applied
}

fn redact_choices_with_messages(
    raw_choices: &[Box<RawValue>],
    sanitized_text: Option<&str>,
    sanitized_messages: &[Fields],
) -> (Vec<Box<RawValue>>, bool) {
    let default_text = sanitized_text
        .filter(|text| !text.is_empty())
        .unwrap_or("[REDACTED]");
    let mut message_iter = sanitized_messages.iter();
    map_items(raw_choices, |choice| {
        let Json::Object(mut message) = Json::field(choice, "message") else {
            return false;
        };
        let mut replacement_text = default_text.to_owned();
        let mut applied = false;
        match message_iter.next() {
            Some(replacement_message) => {
                let text = normalize_content(&Json::field(replacement_message, "content"));
                if !text.is_empty() {
                    replacement_text.clone_from(&text);
                    set_serializable_field(&mut message, "content", &text);
                    applied = true;
                }
            }
            None if Json::field(&message, "content").truthy() => {
                set_serializable_field(&mut message, "content", &replacement_text);
                applied = true;
            }
            None => {}
        }
        if redact_reasoning_fields(&mut message, &replacement_text) {
            applied = true;
        }
        clear_tool_call_arguments(&mut message);
        set_serializable_field(choice, "message", &message);
        applied
    })
}

fn redact_choices_with_text(
    raw_choices: &[Box<RawValue>],
    text: &str,
) -> (Vec<Box<RawValue>>, bool) {
    map_items(raw_choices, |choice| {
        let Json::Object(mut message) = Json::field(choice, "message") else {
            return false;
        };
        let content_applied = redact_field_when_present(&mut message, "content", text);
        let reasoning_applied = redact_reasoning_fields(&mut message, text);
        clear_tool_call_arguments(&mut message);
        set_serializable_field(choice, "message", &message);
        content_applied || reasoning_applied
    })
}

fn redact_reasoning_fields(message: &mut Fields, replacement_text: &str) -> bool {
    if reasoning_parts(message).is_empty() {
        return false;
    }
    set_serializable_field(message, "content", &replacement_text);
    for key in ["reasoning_content", "thinking_blocks", "reasoning_items"] {
        message.shift_remove(key);
    }
    true
}
