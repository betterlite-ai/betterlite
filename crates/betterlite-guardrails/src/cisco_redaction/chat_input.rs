use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::{tool_argument_parts, tool_definition_text};
use crate::cisco_conversation::rewrite::{messages_to_input, rewrite_input_text};
use crate::cisco_conversation::{Fields, normalize_content};
use crate::cisco_redaction::{clear_tool_call_arguments, set_serializable_field, to_raw};

#[must_use]
pub fn redact_chat_input(
    request_data: &mut Fields,
    sanitized_text: Option<&str>,
    sanitized_messages: Option<&[Fields]>,
) -> bool {
    let uses_input = request_data.contains_key("input") && !request_data.contains_key("messages");
    let mut messages = strip_tool_definition_message(request_data, sanitized_messages);
    let has_instructions = !matches!(Json::field(request_data, "instructions"), Json::Null);
    if has_instructions {
        let instructions_redacted =
            redact_responses_instructions(request_data, sanitized_text, messages.as_deref());
        messages = non_instruction_messages(messages);
        if messages.is_none() {
            return instructions_redacted;
        }
    }
    if let Some(message_list) = &messages {
        return rewrite_messages(request_data, uses_input, message_list);
    }
    let Some(text) = sanitized_text.filter(|text| !text.is_empty()) else {
        return false;
    };
    rewrite_with_text(request_data, uses_input, text)
}

// The synthetic tool-definition message echoed back by Cisco has no request field to map to.
fn strip_tool_definition_message(
    request_data: &Fields,
    sanitized_messages: Option<&[Fields]>,
) -> Option<Vec<Fields>> {
    let messages = sanitized_messages.filter(|list| !list.is_empty())?;
    if tool_definition_text(request_data).is_empty() {
        return Some(messages.to_vec());
    }
    let (_, trimmed) = messages.split_last()?;
    (!trimmed.is_empty()).then(|| trimmed.to_vec())
}

fn redact_responses_instructions(
    request_data: &mut Fields,
    sanitized_text: Option<&str>,
    sanitized_messages: Option<&[Fields]>,
) -> bool {
    if let Some(messages) = sanitized_messages.filter(|list| !list.is_empty())
        && let Some(text) = extract_instruction_text(messages)
    {
        set_serializable_field(request_data, "instructions", &text);
        return true;
    }
    let no_conflicting_field = !["input", "messages", "prompt"]
        .iter()
        .any(|key| request_data.contains_key(*key));
    if let Some(text) = sanitized_text.filter(|text| !text.is_empty())
        && no_conflicting_field
    {
        set_serializable_field(request_data, "instructions", &text);
        return true;
    }
    false
}

fn extract_instruction_text(messages: &[Fields]) -> Option<String> {
    messages
        .iter()
        .filter(|message| is_instruction_role(message))
        .find_map(|message| {
            let text = normalize_content(&Json::field(message, "content"));
            (!text.is_empty()).then_some(text)
        })
}

fn non_instruction_messages(messages: Option<Vec<Fields>>) -> Option<Vec<Fields>> {
    let filtered: Vec<Fields> = messages?
        .into_iter()
        .filter(|message| !is_instruction_role(message))
        .collect();
    (!filtered.is_empty()).then_some(filtered)
}

fn is_instruction_role(message: &Fields) -> bool {
    matches!(
        Json::field(message, "role"),
        Json::Text(role) if matches!(role.to_lowercase().as_str(), "system" | "developer")
    )
}

fn rewrite_messages(request_data: &mut Fields, uses_input: bool, messages: &[Fields]) -> bool {
    if uses_input {
        let raw_messages: Vec<Box<RawValue>> = messages.iter().filter_map(to_raw).collect();
        let Some(rewritten) = messages_to_input(&raw_messages) else {
            return false;
        };
        set_serializable_field(request_data, "input", &rewritten);
        return true;
    }
    set_serializable_field(request_data, "messages", &messages.to_vec());
    true
}

fn rewrite_with_text(request_data: &mut Fields, uses_input: bool, text: &str) -> bool {
    if uses_input {
        let original = Json::field(request_data, "input");
        let Some(rewritten) = rewrite_input_text(&original, text) else {
            return false;
        };
        set_serializable_field(request_data, "input", &rewritten);
        return true;
    }
    let redacted_arguments = clear_input_tool_arguments(request_data);
    let redacted_content = redact_last_user_message(request_data, text);
    redacted_content || redacted_arguments
}

fn clear_input_tool_arguments(request_data: &mut Fields) -> bool {
    let Json::List(raw_messages) = Json::field(request_data, "messages") else {
        return false;
    };
    let mut applied = false;
    let mut updated = Vec::with_capacity(raw_messages.len());
    for raw_message in &raw_messages {
        let Json::Object(mut message) = Json::parse(raw_message.get()) else {
            updated.push(raw_message.clone());
            continue;
        };
        if !tool_argument_parts(&message).is_empty() {
            clear_tool_call_arguments(&mut message);
            applied = true;
        }
        updated.push(to_raw(&message).unwrap_or_else(|| raw_message.clone()));
    }
    if applied {
        set_serializable_field(request_data, "messages", &updated);
    }
    applied
}

fn redact_last_user_message(request_data: &mut Fields, text: &str) -> bool {
    let Json::List(raw_messages) = Json::field(request_data, "messages") else {
        return false;
    };
    let target = raw_messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, raw_message)| {
            let Json::Object(message) = Json::parse(raw_message.get()) else {
                return None;
            };
            let is_user =
                matches!(Json::field(&message, "role"), Json::Text(role) if role == "user");
            let has_text_content = matches!(Json::field(&message, "content"), Json::Text(_));
            (is_user && has_text_content).then_some(index)
        });
    let Some(position) = target else {
        return false;
    };
    let mut updated = raw_messages;
    let Some(raw_message) = updated.get(position) else {
        return false;
    };
    let Json::Object(mut message) = Json::parse(raw_message.get()) else {
        return false;
    };
    set_serializable_field(&mut message, "content", &text);
    let rewritten = to_raw(&message).unwrap_or_else(|| raw_message.clone());
    if let Some(slot) = updated.get_mut(position) {
        *slot = rewritten;
    }
    set_serializable_field(request_data, "messages", &updated);
    true
}
