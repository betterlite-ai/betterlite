use super::cache_plan_validation::{cache_ttl, nonempty};
use super::{JsonObject, Position, Section};
use serde_json::{Map, Number, Value};
use sha2::{Digest as _, Sha256};

pub(super) fn blocks(value: &Value) -> Vec<&Value> {
    match value {
        Value::String(_) => vec![value],
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    }
}
pub(super) fn make_position(
    section: Section,
    message_index: i64,
    role: &str,
    value: &Value,
) -> Position {
    let object = value.as_object().cloned().unwrap_or_else(|| {
        let mut item = Map::new();
        item.insert("type".to_owned(), Value::String("text".to_owned()));
        item.insert("text".to_owned(), value.clone());
        item
    });
    let marker = object.get("cache_control").and_then(cache_ttl);
    let block = object
        .into_iter()
        .filter(|(key, _)| key != "cache_control")
        .collect();
    Position {
        section,
        message_index,
        role: role.to_owned(),
        block,
        marker,
    }
}
pub(super) fn cacheable(position: &Position) -> bool {
    match position.block.get("type").and_then(Value::as_str) {
        Some("thinking") => false,
        Some("text") => position
            .block
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(nonempty),
        Some(_) => true,
        None => false,
    }
}
pub(super) fn selected_settings(body: &JsonObject) -> Value {
    let mut settings = Map::new();
    for key in ["thinking", "output_config", "speed"] {
        if let Some(value) = body.get(key) {
            if key != "speed" || value != "standard" {
                settings.insert(key.to_owned(), value.clone());
            }
        }
    }
    Value::Object(settings)
}
pub(super) fn chained_hashes(
    positions: &[Position],
    body: &JsonObject,
    settings: &Value,
) -> Vec<String> {
    let mut previous = digest(settings);
    let mut result = Vec::new();
    for position in positions {
        let current = digest(&Value::Array(vec![
            Value::String(section(position.section).to_owned()),
            Value::Number(Number::from(position.message_index)),
            Value::String(position.role.clone()),
            Value::Object(position.block.clone()),
            if position.section == Section::Messages {
                body.get("tool_choice").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            },
        ]));
        previous = digest(&Value::Array(vec![
            Value::String(previous),
            Value::String(current),
        ]));
        result.push(previous.clone());
    }
    result
}
pub(super) fn section(value: Section) -> &'static str {
    match value {
        Section::Tools => "tools",
        Section::System => "system",
        Section::Messages => "messages",
    }
}
pub(super) fn position_groups(positions: &[Position]) -> Vec<Vec<usize>> {
    let mut result: Vec<Vec<usize>> = Vec::new();
    let mut previous: Option<(Section, i64, String)> = None;
    for (index, position) in positions.iter().enumerate() {
        let kind = position
            .block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let key = (
            position.section,
            position.message_index,
            if kind == "tool_use" || kind == "tool_result" {
                kind.to_owned()
            } else {
                index.to_string()
            },
        );
        if previous.as_ref() != Some(&key) {
            result.push(Vec::new());
            previous = Some(key);
        }
        if let Some(group) = result.last_mut() {
            group.push(index);
        }
    }
    result
}
pub(super) fn lookbacks(
    groups: &[Vec<usize>],
    hashes: &[String],
    index: usize,
    ttl: u32,
    scoped: bool,
) -> Vec<String> {
    groups
        .iter()
        .filter(|group| group[0] <= index)
        .rev()
        .take(20)
        .flat_map(|group| {
            group
                .iter()
                .rev()
                .filter(|item| **item <= index)
                .map(|item| {
                    if scoped {
                        entry_fingerprint(&hashes[*item], ttl)
                    } else {
                        hashes[*item].clone()
                    }
                })
        })
        .collect()
}
pub(super) fn prefix_body(body: &JsonObject, positions: &[Position], last: usize) -> JsonObject {
    let mut result = Map::new();
    for key in ["thinking", "tool_choice", "output_config"] {
        if let Some(value) = body.get(key) {
            result.insert(key.to_owned(), value.clone());
        }
    }
    let tools = positions[..=last]
        .iter()
        .filter(|item| item.section == Section::Tools)
        .map(|item| Value::Object(item.block.clone()))
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        result.insert("tools".to_owned(), Value::Array(tools));
    }
    let system = positions[..=last]
        .iter()
        .filter(|item| item.section == Section::System)
        .map(|item| Value::Object(item.block.clone()))
        .collect::<Vec<_>>();
    if !system.is_empty() {
        result.insert("system".to_owned(), Value::Array(system));
    }
    let mut messages = Vec::new();
    let mut current: Option<(i64, String, Vec<Value>)> = None;
    for item in positions[..=last]
        .iter()
        .filter(|item| item.section == Section::Messages)
    {
        if current
            .as_ref()
            .is_none_or(|value| value.0 != item.message_index)
        {
            if let Some((_, role, content)) = current.take() {
                messages.push(message(&role, content));
            }
            current = Some((item.message_index, item.role.clone(), Vec::new()));
        }
        if let Some((_, _, content)) = current.as_mut() {
            content.push(Value::Object(item.block.clone()));
        }
    }
    if let Some((_, role, content)) = current {
        messages.push(message(&role, content));
    }
    result.insert("messages".to_owned(), Value::Array(messages));
    result
}
pub(super) fn message(role: &str, content: Vec<Value>) -> Value {
    let mut result = Map::new();
    result.insert("role".to_owned(), Value::String(role.to_owned()));
    result.insert("content".to_owned(), Value::Array(content));
    Value::Object(result)
}
pub(super) fn entry_fingerprint(fingerprint: &str, ttl: u32) -> String {
    digest(&Value::Array(vec![
        Value::String("native-cache-prefix-v2".to_owned()),
        Value::String(fingerprint.to_owned()),
        Value::Number(Number::from(ttl)),
    ]))
}
pub(super) fn digest(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
