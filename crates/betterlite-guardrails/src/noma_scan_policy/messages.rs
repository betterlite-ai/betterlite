use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_chat_history::history::{HistoryMessage, convert_history};
use serde_json::value::{RawValue, to_raw_value};

use super::error::NomaError;
use super::request::{as_object, as_object_mut};
use super::wire::{InputMessage, TextPart};

struct ConvertedInput {
    items: Vec<Box<RawValue>>,
    instructions: Option<String>,
}

// The Responses API items to scan, system instructions first; None when there is nothing to scan
pub(super) fn scan_items(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<Vec<Box<RawValue>>>, NomaError> {
    let Some(messages) = messages_from_request(request_data)? else {
        return Ok(None);
    };
    let converted = converted_input(&messages)?;
    let mut items = converted.items;
    if let Some(instructions) = converted
        .instructions
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        let system = InputMessage {
            kind: "message",
            role: "system",
            content: [TextPart {
                kind: "input_text",
                text: instructions,
            }],
        };
        let raw = to_raw_value(&system).map_err(|source| NomaError::PayloadJson { source })?;
        items.insert(0, raw);
    }
    Ok((!items.is_empty()).then_some(items))
}

fn messages_from_request(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<Vec<HistoryMessage>>, NomaError> {
    let messages = match request_data.get("messages") {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(messages)) => messages,
        Some(_) => {
            return Err(NomaError::Field { field: "messages" });
        }
    };
    if messages.is_empty() {
        return Ok(None);
    }
    let mut parsed = Vec::with_capacity(messages.len());
    for message in messages {
        let serialized =
            serde_json::to_value(message).map_err(|source| NomaError::Message { source })?;
        let history =
            serde_json::from_value(serialized).map_err(|source| NomaError::Message { source })?;
        parsed.push(history);
    }
    Ok(Some(parsed))
}

fn converted_input(messages: &[HistoryMessage]) -> Result<ConvertedInput, NomaError> {
    let converted = convert_history(messages).map_err(|source| NomaError::History { source })?;
    let mut items = Vec::with_capacity(converted.input.len());
    for item in &converted.input {
        let raw = to_raw_value(item).map_err(|source| NomaError::PayloadJson { source })?;
        items.push(raw);
    }
    Ok(ConvertedInput {
        items,
        instructions: converted.instructions,
    })
}

// LiteLLM writes the anonymized text over the content of the last user message, in place
pub(super) fn replace_last_user_message(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    content: &str,
) -> Result<(), NomaError> {
    let messages = match request_data.get_mut("messages") {
        None | Some(GuardrailValue::Null) => return Ok(()),
        Some(GuardrailValue::Array(messages)) => messages,
        Some(_) => {
            return Err(NomaError::Field { field: "messages" });
        }
    };
    for message in messages.iter_mut().rev() {
        let Some(fields) = as_object_mut(message) else {
            return Err(NomaError::Field { field: "messages" });
        };
        let is_user =
            matches!(fields.get("role"), Some(GuardrailValue::Text(role)) if role == "user");
        if is_user {
            fields.insert(
                "content".to_owned(),
                GuardrailValue::Text(content.to_owned()),
            );
            return Ok(());
        }
    }
    Ok(())
}

// The first chat choice whose message carries a non-empty string content, as LiteLLM scans
pub(super) fn message_content(response: &GuardrailValue) -> Result<Option<&str>, NomaError> {
    let Some(GuardrailValue::Array(choices)) =
        as_object(response).and_then(|fields| fields.get("choices"))
    else {
        return Ok(None);
    };
    for choice in choices {
        // A choice without a message object is no chat choice, so LiteLLM skips it
        let Some(message) = as_object(choice)
            .and_then(|choice_fields| choice_fields.get("message"))
            .and_then(as_object)
        else {
            continue;
        };
        if let Some(content) = string_field(message, "content", "choices.message.content")?
            .filter(|text| !text.is_empty())
        {
            return Ok(Some(content));
        }
    }
    Ok(None)
}

// LiteLLM replaces content in every choice that has non-empty content
pub(super) fn replace_response_content(response: &mut GuardrailValue, content: &str) {
    let Some(fields) = as_object_mut(response) else {
        return;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get_mut("choices") else {
        return;
    };
    for choice in choices {
        let Some(choice_fields) = as_object_mut(choice) else {
            continue;
        };
        let Some(GuardrailValue::Object(message)) = choice_fields.get_mut("message") else {
            continue;
        };
        let has_content = matches!(
            message.get("content"),
            Some(GuardrailValue::Text(text)) if !text.is_empty()
        );
        if has_content {
            message.insert(
                "content".to_owned(),
                GuardrailValue::Text(content.to_owned()),
            );
        }
    }
}

pub(super) fn response_id(response: &GuardrailValue) -> Result<Option<&str>, NomaError> {
    let Some(fields) = as_object(response) else {
        return Ok(None);
    };
    string_field(fields, "id", "id")
}

fn string_field<'data>(
    fields: &'data BTreeMap<String, GuardrailValue>,
    key: &str,
    field: &'static str,
) -> Result<Option<&'data str>, NomaError> {
    match fields.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(found)) => Ok(Some(found)),
        Some(_) => Err(NomaError::ResponseField { field }),
    }
}
