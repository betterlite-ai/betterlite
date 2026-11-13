#![expect(
    clippy::disallowed_types,
    reason = "raw Responses input items and the chat messages converted from them are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::slice;
use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::response_chat_history::history::{HistoryMessage, convert_history};
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Map, Value};

use error::InputProvenanceError;

pub(super) const TOOL_CALL_ITEM_TYPES: [&str; 2] = ["function_call", "custom_tool_call"];

pub(super) const PATCHABLE_ITEM_FIELDS: [(&str, &str); 3] = [
    ("function_call_output", "output"),
    ("custom_tool_call_output", "output"),
    ("message", "content"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputProvenance {
    pub item_for_message: BTreeMap<usize, usize>,
    pub tainted: BTreeSet<usize>,
}

pub fn input_item_provenance(
    raw_input: &[Value],
    expected_messages: &[Value],
) -> Result<Option<InputProvenance>, InputProvenanceError> {
    if raw_input.iter().any(|item| !item.is_object()) {
        return Ok(None);
    }
    let cache = empty_cache()?;
    let items = to_json_items(raw_input)?;
    let mut solo_conversions = Vec::with_capacity(items.len());
    for item in &items {
        solo_conversions.push(convert(slice::from_ref(item), &cache)?);
    }
    let full_conversion = convert(&items, &cache)?;
    if full_conversion.as_slice() != expected_messages {
        return Ok(None);
    }
    let units = provenance_unit_bounds(raw_input, &solo_conversions);
    let unit_messages = unit_conversions(&items, &units, &solo_conversions, &cache)?;
    if !unit_messages.iter().flatten().eq(full_conversion.iter()) {
        return Ok(None);
    }
    Ok(Some(provenance_maps(&units, &unit_messages)))
}

fn unit_conversions(
    items: &[Json],
    units: &[(usize, usize)],
    solo_conversions: &[Vec<Value>],
    cache: &DualLocalCache,
) -> Result<Vec<Vec<Value>>, InputProvenanceError> {
    let unit_starts: BTreeSet<usize> = units.iter().map(|(start, _)| *start).collect();
    let start_solos = solo_conversions
        .iter()
        .enumerate()
        .filter(|(index, _)| unit_starts.contains(index))
        .map(|(_, solo)| solo);
    let mut unit_messages = Vec::with_capacity(units.len());
    let mut remaining = items.iter();
    // the bounds are contiguous, so each unit's span picks up where the previous one ended
    for ((start, end), solo) in units.iter().zip(start_solos) {
        let span = end.saturating_sub(*start);
        let unit: Vec<Json> = remaining.by_ref().take(span).cloned().collect();
        unit_messages.push(if span == 1 {
            solo.clone()
        } else {
            convert(&unit, cache)?
        });
    }
    Ok(unit_messages)
}

fn provenance_maps(units: &[(usize, usize)], unit_messages: &[Vec<Value>]) -> InputProvenance {
    let mut item_for_message = BTreeMap::new();
    let mut tainted = BTreeSet::new();
    let mut message_offset = 0_usize;
    for ((start, end), messages) in units.iter().zip(unit_messages) {
        let next_offset = message_offset.saturating_add(messages.len());
        if end.saturating_sub(*start) > 1 {
            tainted.extend(message_offset..next_offset);
        } else {
            for message_index in message_offset..next_offset {
                item_for_message.insert(message_index, *start);
            }
        }
        message_offset = next_offset;
    }
    InputProvenance {
        item_for_message,
        tainted,
    }
}

#[must_use]
pub fn provenance_unit_bounds(
    raw_input: &[Value],
    solo_conversions: &[Vec<Value>],
) -> Vec<(usize, usize)> {
    let mut roles: Vec<Option<String>> = Vec::with_capacity(solo_conversions.len());
    let mut last: Option<String> = None;
    for messages in solo_conversions {
        if let Some(role) = last_message_role(messages) {
            last = Some(role.to_owned());
        }
        roles.push(last.clone());
    }
    let mut starts = Vec::new();
    for (index, item) in raw_input.iter().enumerate() {
        let joins_assistant = index != 0
            && is_tool_call_item(item)
            && roles
                .get(index.saturating_sub(1))
                .is_some_and(|role| role.as_deref() == Some("assistant"));
        if !joins_assistant {
            starts.push(index);
        }
    }
    let mut bounds = Vec::with_capacity(starts.len());
    for (position, start) in starts.iter().enumerate() {
        let end = starts
            .get(position.saturating_add(1))
            .copied()
            .unwrap_or(raw_input.len());
        bounds.push((*start, end));
    }
    bounds
}

#[must_use]
pub fn last_message_role(messages: &[Value]) -> Option<&str> {
    messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
}

#[must_use]
pub fn is_tool_call_item(item: &Value) -> bool {
    item.as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| TOOL_CALL_ITEM_TYPES.contains(&kind))
}

#[must_use]
pub fn tool_call_output_item_mapping(item: &Value) -> Option<&Map<String, Value>> {
    let object = item.as_object()?;
    let kind = object.get("type")?.as_str()?;
    TOOL_CALL_ITEM_TYPES.contains(&kind).then_some(object)
}

#[must_use]
pub fn is_tool_call_output_item(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    let Some(kind) = object.get("type").and_then(Value::as_str) else {
        return false;
    };
    TOOL_CALL_ITEM_TYPES.contains(&kind)
}

#[must_use]
pub fn item_rewrite_field(item: &Map<String, Value>) -> Option<&'static str> {
    match item.get("type") {
        None | Some(Value::Null) => item.contains_key("content").then_some("content"),
        Some(Value::String(kind)) => PATCHABLE_ITEM_FIELDS
            .iter()
            .find_map(|(name, field)| (*name == kind.as_str()).then_some(*field)),
        Some(_) => None,
    }
}

pub fn rewritten_input_item(
    item: &Map<String, Value>,
    rewritten: &Value,
) -> Result<Option<Map<String, Value>>, InputProvenanceError> {
    let Some(field) = item_rewrite_field(item) else {
        return Ok(None);
    };
    let Some(rewritten_object) = rewritten.as_object() else {
        return Ok(None);
    };
    if let (Some(Value::String(_)), Some(Value::String(replacement))) =
        (item.get(field), rewritten_object.get("content"))
    {
        let mut patched = item.clone();
        patched.insert(field.to_owned(), Value::String(replacement.clone()));
        return Ok(Some(patched));
    }
    let converted = convert_chat_row(rewritten)?;
    if converted.len() != 1 {
        return Ok(None);
    }
    let Some(converted_object) = converted.first().and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(converted_value) = converted_object.get(field) else {
        return Ok(None);
    };
    if converted_value.is_null() {
        return Ok(None);
    }
    let mut patched = item.clone();
    patched.insert(field.to_owned(), converted_value.clone());
    Ok(Some(patched))
}

fn convert_chat_row(row: &Value) -> Result<Vec<Value>, InputProvenanceError> {
    let message: HistoryMessage = serde_json::from_value(row.clone())
        .map_err(|source| InputProvenanceError::ChatRow { source })?;
    let converted = convert_history(&[message])?;
    Ok(converted.input)
}

fn convert(items: &[Json], cache: &DualLocalCache) -> Result<Vec<Value>, InputProvenanceError> {
    let request = InputRequest {
        input: Some(Json::List(items.to_vec())),
        instructions: None,
    };
    input_messages(&request, false, ApiLimits::default(), cache)
        .map_err(|source| InputProvenanceError::Conversion { source })
}

fn to_json_items(raw_input: &[Value]) -> Result<Vec<Json>, InputProvenanceError> {
    let mut items = Vec::with_capacity(raw_input.len());
    for item in raw_input {
        let converted: Json = serde_json::from_value(item.clone())
            .map_err(|source| InputProvenanceError::InputItem { source })?;
        items.push(converted);
    }
    Ok(items)
}

fn empty_cache() -> Result<DualLocalCache, InputProvenanceError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| InputProvenanceError::Redactor { source })?;
    let diagnostics = Diagnostics {
        redactor: Arc::new(redactor),
        emit: Arc::new(|_event| {}),
    };
    Ok(DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    ))
}
