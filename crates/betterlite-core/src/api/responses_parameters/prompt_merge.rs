use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesParametersError;

pub const NO_INPUT_MESSAGES_WARNING: &str = "Prompt management hook returned messages without Responses API input messages; merged messages were ignored";
pub const NON_MESSAGE_ITEMS_WARNING: &str =
    "Prompt management hook replaced Responses API messages; non-message input items were dropped";

static ABSENT_FIELD: JsonValue = JsonValue::Null;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientMessage {
    pub position: Option<usize>,
    pub message: JsonValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnedMessage {
    pub source: Option<usize>,
    pub message: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMergeWarning {
    NoInputMessages,
    NonMessageItemsDropped,
}

impl PromptMergeWarning {
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::NoInputMessages => NO_INPUT_MESSAGES_WARNING,
            Self::NonMessageItemsDropped => NON_MESSAGE_ITEMS_WARNING,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptMergeOutcome {
    pub input: JsonValue,
    pub warning: Option<PromptMergeWarning>,
}

pub fn input_to_chat_messages(
    input: Option<&JsonValue>,
) -> Result<Vec<ClientMessage>, ResponsesParametersError> {
    match input {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Text(text)) => Ok(vec![ClientMessage {
            position: None,
            message: user_message(text),
        }]),
        Some(JsonValue::Array(items)) => {
            Ok(items.iter().enumerate().filter_map(message_at).collect())
        }
        Some(JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Object(_)) => {
            Err(ResponsesParametersError::NotTextOrList { field: "input" })
        }
    }
}

fn message_at((index, item): (usize, &JsonValue)) -> Option<ClientMessage> {
    let JsonValue::Object(fields) = item else {
        return None;
    };
    fields.contains_key("role").then(|| ClientMessage {
        position: Some(index),
        message: item.clone(),
    })
}

fn user_message(text: &str) -> JsonValue {
    JsonValue::Object(IndexMap::from([
        ("role".to_owned(), JsonValue::Text("user".to_owned())),
        ("content".to_owned(), JsonValue::Text(text.to_owned())),
    ]))
}

#[must_use]
pub fn shape_prompt_managed_message(message: &JsonValue) -> JsonValue {
    let JsonValue::Object(fields) = message else {
        return message.clone();
    };
    if matches!(fields.get("role"), Some(JsonValue::Text(role)) if role == "assistant") {
        return message.clone();
    }
    let Some(JsonValue::Array(content)) = fields.get("content") else {
        return message.clone();
    };
    if !content.iter().any(|part| {
        matches!(part, JsonValue::Object(part_fields) if matches!(part_fields.get("type"), Some(JsonValue::Text(kind)) if kind == "text"))
    }) {
        return message.clone();
    }
    let shaped: Vec<JsonValue> = content.iter().map(as_input_text_part).collect();
    let mut copy = fields.clone();
    copy.insert("content".to_owned(), JsonValue::Array(shaped));
    JsonValue::Object(copy)
}

fn as_input_text_part(part: &JsonValue) -> JsonValue {
    let JsonValue::Object(fields) = part else {
        return part.clone();
    };
    if !matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "text") {
        return part.clone();
    }
    let mut copy = fields.clone();
    copy.insert("type".to_owned(), JsonValue::Text("input_text".to_owned()));
    JsonValue::Object(copy)
}

pub fn merge_prompt_management_input(
    original_input: &JsonValue,
    merged_input: &[ReturnedMessage],
) -> Result<PromptMergeOutcome, ResponsesParametersError> {
    if matches!(original_input, JsonValue::Text(_)) {
        return Ok(PromptMergeOutcome {
            input: shaped_sequence(merged_input),
            warning: None,
        });
    }
    let JsonValue::Array(original_items) = original_input else {
        return Err(ResponsesParametersError::NotTextOrList { field: "input" });
    };
    let mut client = input_to_chat_messages(Some(original_input))?;
    apply_hook_mutations(&mut client, merged_input)?;
    let positions: Vec<usize> = client.iter().filter_map(|entry| entry.position).collect();
    let (input, warning) = if positions.len() == original_items.len() {
        (shaped_sequence(merged_input), None)
    } else if positions.is_empty() {
        (
            original_input.clone(),
            Some(PromptMergeWarning::NoInputMessages),
        )
    } else if corresponding_messages(&client, merged_input) {
        (
            corresponding_input(original_items, &positions, merged_input),
            None,
        )
    } else if all_messages_preserved(client.len(), merged_input) {
        (
            preserved_input(original_items, &client, merged_input)?,
            None,
        )
    } else {
        (
            shaped_sequence(merged_input),
            Some(PromptMergeWarning::NonMessageItemsDropped),
        )
    };
    Ok(PromptMergeOutcome { input, warning })
}

fn apply_hook_mutations(
    client: &mut [ClientMessage],
    merged_input: &[ReturnedMessage],
) -> Result<(), ResponsesParametersError> {
    for returned in merged_input {
        let Some(source) = returned.source else {
            continue;
        };
        let Some(entry) = client.get_mut(source) else {
            return Err(ResponsesParametersError::MessageSource { index: source });
        };
        entry.message = returned.message.clone();
    }
    Ok(())
}

fn corresponding_messages(client: &[ClientMessage], merged_input: &[ReturnedMessage]) -> bool {
    client.len() == merged_input.len()
        && client.iter().zip(merged_input).all(|(original, returned)| {
            field(&original.message, "role") == field(&returned.message, "role")
                && (!matches!(field(&original.message, "id"), JsonValue::Text(_))
                    || field(&original.message, "id") == field(&returned.message, "id"))
        })
}

fn field<'value>(message: &'value JsonValue, name: &str) -> &'value JsonValue {
    let JsonValue::Object(fields) = message else {
        return &ABSENT_FIELD;
    };
    fields.get(name).unwrap_or(&ABSENT_FIELD)
}

fn corresponding_input(
    original_items: &[JsonValue],
    positions: &[usize],
    merged_input: &[ReturnedMessage],
) -> JsonValue {
    let mut replacements: BTreeMap<usize, &JsonValue> = positions
        .iter()
        .copied()
        .zip(merged_input.iter().map(|returned| &returned.message))
        .collect();
    let items = original_items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            replacements
                .remove(&index)
                .map_or_else(|| item.clone(), shape_prompt_managed_message)
        })
        .collect();
    JsonValue::Array(items)
}

fn all_messages_preserved(client_len: usize, merged_input: &[ReturnedMessage]) -> bool {
    (0..client_len).all(|index| {
        merged_input
            .iter()
            .any(|returned| returned.source == Some(index))
    })
}

fn preserved_input(
    original_items: &[JsonValue],
    client: &[ClientMessage],
    merged_input: &[ReturnedMessage],
) -> Result<JsonValue, ResponsesParametersError> {
    let mut prefixes: BTreeMap<usize, Vec<JsonValue>> = BTreeMap::new();
    let mut previous: Option<usize> = None;
    for entry in client {
        let Some(position) = entry.position else {
            continue;
        };
        let start = previous.map_or(0, |index| index.saturating_add(1));
        let prefix = original_items
            .get(start..position)
            .ok_or(ResponsesParametersError::OriginalItems)?;
        prefixes.insert(position, prefix.to_vec());
        previous = Some(position);
    }
    let last_position = previous.ok_or(ResponsesParametersError::OriginalItems)?;
    let trailing = original_items
        .get(last_position.saturating_add(1)..)
        .ok_or(ResponsesParametersError::OriginalItems)?;
    let mut items = Vec::new();
    for returned in merged_input {
        if let Some(source) = returned.source
            && let Some(position) = client.get(source).and_then(|entry| entry.position)
            && let Some(prefix) = prefixes.get(&position)
        {
            items.extend(prefix.iter().cloned());
        }
        items.push(shape_prompt_managed_message(&returned.message));
    }
    items.extend(trailing.iter().cloned());
    Ok(JsonValue::Array(items))
}

fn shaped_sequence(merged_input: &[ReturnedMessage]) -> JsonValue {
    JsonValue::Array(
        merged_input
            .iter()
            .map(|returned| shape_prompt_managed_message(&returned.message))
            .collect(),
    )
}
