#![expect(
    clippy::disallowed_types,
    reason = "guardrail-returned chat rows and the Anthropic request body are provider JSON that betterlite does not model"
)]

mod conversion;
mod deps;
pub mod error;
mod system_rows;
mod tests;

use std::collections::HashSet;

use betterlite_core::api::messages_chat_transform::compose::translate_anthropic_to_openai;
use betterlite_core::api::messages_chat_transform::settings::AdapterSettings;
use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use serde_json::{Map, Value};

use conversion::Conversion;
use deps::{NoCapabilities, NoParameters};
use error::StructuredWriteBackError;
use system_rows::{is_present, text_block, text_block_row};

const CACHE_CONTROL: &str = "cache_control";
const CONTENT: &str = "content";
const MESSAGES: &str = "messages";
const MODEL: &str = "model";
const ROLE: &str = "role";
const SYSTEM: &str = "system";
const TEXT: &str = "text";
const THINKING: &str = "thinking";
const TYPE: &str = "type";

#[must_use]
pub fn is_system(message: &Value) -> bool {
    message
        .get(ROLE)
        .and_then(Value::as_str)
        .is_some_and(|role| role.eq_ignore_ascii_case(SYSTEM))
}

pub async fn translate_to_openai(
    data: &Map<String, Value>,
) -> Result<Object, StructuredWriteBackError> {
    let mut request: Object = serde_json::from_value(Value::Object(data.clone()))?;
    let parameters = NoParameters;
    let settings = AdapterSettings {
        preserve_midturn_system: true,
        ..AdapterSettings::new(&parameters)
    };
    let mut predicates = NoCapabilities;
    let (translated, _tool_name_mapping) =
        translate_anthropic_to_openai(&mut request, &settings, &mut predicates).await?;
    Ok(translated)
}

pub async fn hoisted_system_message(
    data: &Map<String, Value>,
) -> Result<Option<Value>, StructuredWriteBackError> {
    let Some(system) = data.get(SYSTEM).filter(|value| is_present(value)) else {
        return Ok(None);
    };
    let mut probe = Map::new();
    probe.insert(MODEL.to_owned(), Value::String(model_of(data)));
    probe.insert(MESSAGES.to_owned(), Value::Array(Vec::new()));
    probe.insert(SYSTEM.to_owned(), system.clone());
    let reply = translate_to_openai(&probe).await?;
    let translated = serde_json::to_value(&reply)?;
    let first = translated
        .get(MESSAGES)
        .and_then(Value::as_array)
        .and_then(|messages| messages.first());
    Ok(first.cloned())
}

#[must_use]
pub fn system_row_to_anthropic(message: &Value) -> Option<Map<String, Value>> {
    let content = match message.get(CONTENT)? {
        Value::String(text) if !text.is_empty() => Value::String(text.clone()),
        Value::Array(blocks) => {
            let converted: Vec<Value> = blocks.iter().filter_map(text_block_row).collect();
            if converted.is_empty() {
                return None;
            }
            Value::Array(converted)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return None;
        }
    };
    let mut row = Map::new();
    row.insert(ROLE.to_owned(), Value::String(SYSTEM.to_owned()));
    row.insert(CONTENT.to_owned(), content);
    Some(row)
}

pub fn fold_leading_systems(
    data: &mut Map<String, Value>,
    leading_systems: &[Value],
    include_existing_system: bool,
) {
    let mut folded: Vec<Value> = Vec::new();
    if include_existing_system && let Some(existing) = data.get(SYSTEM) {
        match existing {
            Value::String(text) if !text.is_empty() => folded.push(text_block(text.clone())),
            Value::Array(blocks) => folded.extend(blocks.iter().cloned()),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Object(_) => {}
        }
    }
    for row in leading_systems {
        let Some(converted) = system_row_to_anthropic(row) else {
            continue;
        };
        match converted.get(CONTENT) {
            Some(Value::String(text)) => folded.push(text_block(text.clone())),
            Some(Value::Array(blocks)) => folded.extend(blocks.iter().cloned()),
            Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {}
        }
    }
    if folded.is_empty() {
        data.remove(SYSTEM);
        return;
    }
    data.insert(SYSTEM.to_owned(), Value::Array(folded));
}

#[must_use]
pub fn defer_tool_exchange_systems(messages: &[Value]) -> Vec<Value> {
    let positioned: Vec<(usize, PromptJson)> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| !is_system(message))
        .map(|(index, message)| {
            // Value and prompts Json share one data model, so a row that cannot convert is still emitted.
            let row =
                serde_json::from_value::<PromptJson>(message.clone()).unwrap_or(PromptJson::Null);
            (index, row)
        })
        .collect();
    let rows: Vec<PromptJson> = positioned.iter().map(|(_, row)| row.clone()).collect();
    let mut deferred_positions: HashSet<usize> = HashSet::new();
    let mut exchange_ends: HashSet<usize> = HashSet::new();
    for group in group_tool_exchanges(&rows) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        // The groups index the non-system rows, so both lookups always find their position.
        let (Some((start, _)), Some((end, _))) = (positioned.get(*first), positioned.get(*last))
        else {
            continue;
        };
        exchange_ends.insert(*end);
        deferred_positions.extend(start.saturating_add(1)..*end);
    }
    let held_positions: HashSet<usize> = deferred_positions
        .into_iter()
        .filter(|index| messages.get(*index).is_some_and(is_system))
        .collect();
    reorder_messages(messages, &held_positions, &exchange_ends)
}

fn reorder_messages(
    messages: &[Value],
    held_positions: &HashSet<usize>,
    exchange_ends: &HashSet<usize>,
) -> Vec<Value> {
    let mut ordered: Vec<Value> = Vec::with_capacity(messages.len());
    let mut held: Vec<Value> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if held_positions.contains(&index) {
            held.push(message.clone());
            continue;
        }
        ordered.push(message.clone());
        if exchange_ends.contains(&index) {
            ordered.append(&mut held);
        }
    }
    ordered.append(&mut held);
    ordered
}

pub async fn write_back_structured_messages(
    data: &mut Map<String, Value>,
    structured_messages: &[Value],
    hoisted_system_message: Option<&Value>,
    preserve_system_messages: bool,
) -> Result<(), StructuredWriteBackError> {
    let model = model_of(data);
    let ordered = defer_tool_exchange_systems(structured_messages);
    let leading_count = ordered
        .iter()
        .position(|message| !is_system(message))
        .unwrap_or(ordered.len());
    let (leading, rest) = ordered.split_at(leading_count);
    // LiteLLM also matches by object identity, and equal JSON rows cover every match it can make.
    let hoisted_in_leading = leading
        .iter()
        .any(|message| hoisted_system_message.is_some_and(|hoisted| message == hoisted));
    // Folding an untouched hoisted prompt would rebuild the client's system string as text blocks.
    let only_the_hoisted_prompt = leading_count == 1 && hoisted_in_leading;
    if !leading.is_empty() && !only_the_hoisted_prompt {
        fold_leading_systems(data, leading, hoisted_system_message.is_none());
    }
    let mut conversion = Conversion::new(&model, hoisted_system_message, preserve_system_messages);
    conversion.hoisted_dropped = hoisted_in_leading;
    conversion.walk(rest).await?;
    let converted = conversion.finish().await?;
    data.insert(MESSAGES.to_owned(), Value::Array(converted));
    Ok(())
}

fn model_of(data: &Map<String, Value>) -> String {
    match data.get(MODEL) {
        Some(Value::String(text)) => text.clone(),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        )
        | None => String::new(),
    }
}
