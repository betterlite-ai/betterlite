use std::collections::BTreeSet;

use betterlite_compression::protected_indices::get_protected_indices;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::arguments::json::Json;
use thiserror::Error;

use crate::content_text::{content_to_text, is_all_text_parts, merge_rewritten_text_parts};
use crate::headroom_settings::HEADROOM_RETRIEVE_TOOL_NAME;
use crate::primitives::json_value::JsonValue;

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeadroomMessagesError {
    #[error("headroom compression returned {returned} rows for {sent} rows sent")]
    ShortCompressed { sent: usize, returned: usize },
}

#[must_use]
pub fn flatten_for_compression(rows: &[JsonValue]) -> Vec<JsonValue> {
    let mut flattened = Vec::with_capacity(rows.len());
    for row in rows {
        flattened.push(flatten_row(row));
    }
    flattened
}

fn flatten_row(row: &JsonValue) -> JsonValue {
    let JsonValue::Object(fields) = row else {
        return row.clone();
    };
    let Some(content) = fields.get("content") else {
        return row.clone();
    };
    let converted = serde_json::to_value(content)
        .ok()
        .and_then(|encoded| serde_json::from_value::<GuardrailValue>(encoded).ok());
    let Some(guardrail_content) = converted else {
        return row.clone();
    };
    if !is_all_text_parts(&guardrail_content) {
        return row.clone();
    }
    let text = content_to_text(&guardrail_content);
    if text.is_empty() {
        return row.clone();
    }
    let mut flattened = fields.clone();
    flattened.insert("content".to_owned(), JsonValue::String(text));
    JsonValue::Object(flattened)
}

#[must_use]
pub fn restore_content_shapes(originals: &[JsonValue], returned: &[JsonValue]) -> Vec<JsonValue> {
    let roles_match = originals.len() == returned.len()
        && originals
            .iter()
            .zip(returned)
            .all(|(original, restored)| field(original, "role") == field(restored, "role"));
    if !roles_match {
        return returned.to_vec();
    }
    originals
        .iter()
        .zip(returned)
        .map(|(original, restored)| restore_row(original, restored))
        .collect()
}

fn restore_row(original: &JsonValue, returned: &JsonValue) -> JsonValue {
    let (Some(original_content), Some(JsonValue::String(returned_text))) =
        (field(original, "content"), field(returned, "content"))
    else {
        return returned.clone();
    };
    let converted = serde_json::to_value(original_content)
        .ok()
        .and_then(|encoded| serde_json::from_value::<GuardrailValue>(encoded).ok());
    let Some(guardrail_content) = converted else {
        return returned.clone();
    };
    let GuardrailValue::Array(parts) = &guardrail_content else {
        return returned.clone();
    };
    let content = if content_to_text(&guardrail_content) == *returned_text {
        original_content.clone()
    } else {
        let Ok(merged) = serde_json::to_value(merge_rewritten_text_parts(parts, returned_text))
        else {
            return returned.clone();
        };
        let Ok(merged_content) = serde_json::from_value::<JsonValue>(merged) else {
            return returned.clone();
        };
        merged_content
    };
    let JsonValue::Object(mut fields) = returned.clone() else {
        return returned.clone();
    };
    fields.insert("content".to_owned(), content);
    JsonValue::Object(fields)
}

pub fn restore_protected_messages(
    messages: &[JsonValue],
    compressed: &[JsonValue],
    protected: &BTreeSet<usize>,
) -> Result<Vec<JsonValue>, HeadroomMessagesError> {
    let sent = (0..messages.len())
        .filter(|index| !protected.contains(index))
        .count();
    if compressed.len() < sent {
        return Err(HeadroomMessagesError::ShortCompressed {
            sent,
            returned: compressed.len(),
        });
    }
    let mut compressed_rows = compressed.iter();
    let mut restored = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        if protected.contains(&index) {
            restored.push(message.clone());
            continue;
        }
        let Some(returned) = compressed_rows.next() else {
            return Err(HeadroomMessagesError::ShortCompressed {
                sent,
                returned: compressed.len(),
            });
        };
        restored.push(returned.clone());
    }
    Ok(restored)
}

#[must_use]
pub fn retrieve_call_ids(messages: Option<&JsonValue>) -> BTreeSet<String> {
    let Some(JsonValue::Array(rows)) = messages else {
        return BTreeSet::new();
    };
    let mut ids = BTreeSet::new();
    for row in rows {
        ids.extend(retrieve_ids_in_row(row, RetrieveSource::OpenAi));
        ids.extend(retrieve_ids_in_row(row, RetrieveSource::Anthropic));
    }
    ids
}

#[must_use]
pub fn retrieval_result_indices(
    messages: &[JsonValue],
    extra_ids: &BTreeSet<String>,
) -> BTreeSet<usize> {
    let mut retrieve_ids = extra_ids.clone();
    for row in messages {
        retrieve_ids.extend(retrieve_ids_in_row(row, RetrieveSource::OpenAi));
    }
    if retrieve_ids.is_empty() {
        return BTreeSet::new();
    }
    messages
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            matches!(
                field(row, "role").and_then(json_str),
                Some("tool" | "function")
            )
        })
        .filter(|(_, row)| {
            field(row, "tool_call_id")
                .and_then(json_str)
                .is_some_and(|id| retrieve_ids.contains(id))
        })
        .map(|(index, _)| index)
        .collect()
}

#[must_use]
pub fn protected_indices(messages: &[JsonValue], extra_ids: &BTreeSet<String>) -> BTreeSet<usize> {
    let Ok(encoded) = serde_json::to_value(messages) else {
        tracing::warn!("headroom: messages did not encode as JSON");
        return BTreeSet::new();
    };
    let Some(rows) = encoded.as_array() else {
        return BTreeSet::new();
    };
    let mut protected: BTreeSet<usize> = get_protected_indices(rows).into_iter().collect();
    protected.extend(retrieval_result_indices(messages, extra_ids));
    // Json is untagged over every JSON value, so the conversion only fails for a row that is not JSON.
    if let Ok(prompt_messages) = serde_json::from_value::<Vec<Json>>(encoded.clone()) {
        for group in group_tool_exchanges(&prompt_messages) {
            if group.iter().any(|member| protected.contains(member)) {
                protected.extend(group);
            }
        }
    } else {
        tracing::warn!("headroom: messages did not decode as prompt messages");
    }
    for (index, row) in messages.iter().enumerate() {
        if field(row, "role").and_then(json_str) == Some("assistant") {
            protected.insert(index);
        }
    }
    protected
}

#[derive(Clone, Copy)]
enum RetrieveSource {
    OpenAi,
    Anthropic,
}

fn retrieve_ids_in_row(row: &JsonValue, source: RetrieveSource) -> Vec<String> {
    match source {
        RetrieveSource::OpenAi => {
            if field(row, "role").and_then(json_str) != Some("assistant") {
                return Vec::new();
            }
            let Some(JsonValue::Array(tool_calls)) = field(row, "tool_calls") else {
                return Vec::new();
            };
            tool_calls
                .iter()
                .filter_map(|tool_call| {
                    let function = field(tool_call, "function")
                        .and_then(|function| field(function, "name"))
                        .and_then(json_str)?;
                    let id = field(tool_call, "id").and_then(json_str)?;
                    if id.is_empty() || !is_retrieve_tool_name(function) {
                        return None;
                    }
                    Some(id.to_owned())
                })
                .collect()
        }
        RetrieveSource::Anthropic => {
            let Some(JsonValue::Array(blocks)) = field(row, "content") else {
                return Vec::new();
            };
            blocks
                .iter()
                .filter(|block| field(block, "type").and_then(json_str) == Some("tool_use"))
                .filter(|block| {
                    field(block, "name")
                        .and_then(json_str)
                        .is_some_and(is_retrieve_tool_name)
                })
                .filter_map(|block| field(block, "id").and_then(json_str).map(str::to_owned))
                .collect()
        }
    }
}

fn is_retrieve_tool_name(name: &str) -> bool {
    name == HEADROOM_RETRIEVE_TOOL_NAME
        || name.ends_with(&format!("__{HEADROOM_RETRIEVE_TOOL_NAME}"))
}

fn field<'row>(row: &'row JsonValue, name: &str) -> Option<&'row JsonValue> {
    match row {
        JsonValue::Object(fields) => fields.get(name),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn json_str(value: &JsonValue) -> Option<&str> {
    match value {
        JsonValue::String(text) => Some(text),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}
