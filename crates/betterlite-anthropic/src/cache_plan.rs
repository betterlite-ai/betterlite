#![expect(
    clippy::restriction,
    clippy::pedantic,
    clippy::style,
    clippy::complexity,
    clippy::nursery,
    reason = "Anthropic cache plans preserve arbitrary provider JSON and insertion order for exact fingerprints"
)]
use serde_json::{Map, Number, Value};
use sha2::{Digest as _, Sha256};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub type JsonObject = Map<String, Value>;
pub const DEFAULT_ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheBoundary {
    pub fingerprint: String,
    pub prefix_body: JsonObject,
    pub ttl_seconds: u32,
    pub lookback_fingerprints: Vec<String>,
    pub content_fingerprint: String,
    pub lookback_content_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptCachePlan {
    pub full_body: JsonObject,
    pub breakpoints: Vec<CacheBoundary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedCacheReason {
    UnsupportedPromptShape,
    ConflictingCacheTtl,
    TooManyCacheBreakpoints,
    InvalidCacheTtlOrder,
    UnsupportedThinkingCacheSemantics,
    TokenCountUnavailable,
    InconsistentPrefixTokenCount,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedCachePlan {
    pub reason: UnsupportedCacheReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptPrefix {
    pub prefix_body: JsonObject,
    pub fingerprint: String,
    pub fingerprints: Vec<String>,
    pub ttl_seconds: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Section {
    Tools,
    System,
    Messages,
}

#[derive(Clone, Debug)]
struct Position {
    section: Section,
    message_index: i64,
    role: String,
    block: JsonObject,
    marker: Option<u32>,
}

pub fn parse_cache_plan(body: &JsonObject) -> Result<PromptCachePlan, UnsupportedCachePlan> {
    if !valid_shape(body, true) {
        return Err(unsupported(UnsupportedCacheReason::UnsupportedPromptShape));
    }
    let positions = positions(body);
    let markers = markers(body, &positions)?;
    let hashes = chained_hashes(&positions, body, &selected_settings(body));
    let groups = position_groups(&positions);
    let mut breakpoints = Vec::new();
    for (index, ttl) in markers {
        let Some(hash) = hashes.get(index) else {
            return Err(unsupported(UnsupportedCacheReason::UnsupportedPromptShape));
        };
        breakpoints.push(CacheBoundary {
            fingerprint: entry_fingerprint(hash, ttl),
            prefix_body: prefix_body(body, &positions, index),
            ttl_seconds: ttl,
            lookback_fingerprints: lookbacks(&groups, &hashes, index, ttl, true),
            content_fingerprint: hash.clone(),
            lookback_content_fingerprints: lookbacks(&groups, &hashes, index, ttl, false),
        });
    }
    Ok(PromptCachePlan {
        full_body: body.clone(),
        breakpoints,
    })
}

fn markers(
    body: &JsonObject,
    positions: &[Position],
) -> Result<Vec<(usize, u32)>, UnsupportedCachePlan> {
    let automatic = positions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| cacheable(item));
    let mut result: Vec<(usize, u32)> = positions
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.marker.map(|ttl| (index, ttl)))
        .collect();
    if let Some(request_ttl) = body.get("cache_control").and_then(cache_ttl) {
        if let Some((index, item)) = automatic {
            if let Some(position_ttl) = item.marker {
                if position_ttl != request_ttl {
                    return Err(unsupported(UnsupportedCacheReason::ConflictingCacheTtl));
                }
            } else {
                result.push((index, request_ttl));
            }
        }
    }
    result.sort_by_key(|item| item.0);
    if result.len() > 4 {
        return Err(unsupported(UnsupportedCacheReason::TooManyCacheBreakpoints));
    }
    if result.windows(2).any(|pair| {
        pair.first().map(|item| item.1).unwrap_or(0) < pair.last().map(|item| item.1).unwrap_or(0)
    }) {
        return Err(unsupported(UnsupportedCacheReason::InvalidCacheTtlOrder));
    }
    Ok(result)
}

#[must_use]
pub fn parse_prompt(body: &JsonObject) -> Option<PromptPrefix> {
    if !valid_shape(body, false) {
        return None;
    }
    let plan = parse_cache_plan(body).ok()?;
    if plan.breakpoints.len() != 1 {
        return None;
    }
    let boundary = plan.breakpoints.first()?;
    Some(PromptPrefix {
        prefix_body: boundary.prefix_body.clone(),
        fingerprint: boundary.fingerprint.clone(),
        fingerprints: boundary.lookback_fingerprints.clone(),
        ttl_seconds: boundary.ttl_seconds,
    })
}

#[must_use]
pub fn cache_scope(
    caller_key_hash: &str,
    deployment_id: &str,
    provider_key: &str,
    model: &str,
    anthropic_version: &str,
) -> String {
    digest(&Value::Array(
        [
            caller_key_hash,
            deployment_id,
            provider_key,
            model,
            anthropic_version,
        ]
        .into_iter()
        .map(|item| Value::String(item.to_owned()))
        .collect(),
    ))
}

const fn unsupported(reason: UnsupportedCacheReason) -> UnsupportedCachePlan {
    UnsupportedCachePlan { reason }
}

fn valid_shape(body: &JsonObject, plan: bool) -> bool {
    let common = [
        "model",
        "max_tokens",
        "stream",
        "temperature",
        "top_p",
        "top_k",
        "stop_sequences",
        "metadata",
        "messages",
        "system",
        "tools",
    ];
    let extras = [
        "cache_control",
        "thinking",
        "tool_choice",
        "output_config",
        "speed",
        "service_tier",
    ];
    if !body.keys().all(|key| {
        common.contains(&key.as_str())
            || (plan && extras.contains(&key.as_str()))
            || (!plan && false)
    }) {
        return false;
    }
    if !valid_options(body)
        || !body.get("messages").is_some_and(|value| {
            value.as_array().is_some_and(|items| {
                !items.is_empty() && items.iter().all(|item| valid_message(item, plan))
            })
        })
    {
        return false;
    }
    if let Some(system) = body.get("system") {
        if !valid_system(system, plan) {
            return false;
        }
    }
    if let Some(tools) = body.get("tools") {
        if !tools
            .as_array()
            .is_some_and(|items| items.iter().all(|item| valid_tool(item, plan)))
        {
            return false;
        }
    }
    if let Some(control) = body.get("cache_control") {
        if !plan || cache_ttl(control).is_none() {
            return false;
        }
    }
    if plan {
        if !valid_plan_options(body) {
            return false;
        }
    }
    true
}

fn valid_options(body: &JsonObject) -> bool {
    body.get("model").is_none_or(Value::is_string)
        && body.get("max_tokens").is_none_or(integer)
        && body.get("stream").is_none_or(Value::is_boolean)
        && body.get("temperature").is_none_or(number)
        && body.get("top_p").is_none_or(number)
        && body.get("top_k").is_none_or(integer)
        && body.get("stop_sequences").is_none_or(|value| {
            value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        })
        && body.get("metadata").is_none_or(Value::is_object)
}

fn valid_plan_options(body: &JsonObject) -> bool {
    body.get("thinking").is_none_or(Value::is_object)
        && body.get("tool_choice").is_none_or(Value::is_object)
        && body.get("output_config").is_none_or(Value::is_object)
        && body.get("speed").is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|text| text == "fast" || text == "standard")
        })
        && body.get("service_tier").is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|text| text == "auto" || text == "standard_only")
        })
}

fn valid_message(value: &Value, plan: bool) -> bool {
    let Some(message) = value.as_object() else {
        return false;
    };
    let roles = if plan {
        ["user", "assistant", "system"].as_slice()
    } else {
        ["user", "assistant"].as_slice()
    };
    message.keys().all(|key| key == "role" || key == "content")
        && message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| roles.contains(&role))
        && message.get("content").is_some_and(|content| {
            content.as_str().is_some_and(nonempty)
                || content
                    .as_array()
                    .is_some_and(|items| items.iter().all(|item| valid_block(item, plan, false)))
        })
}

fn valid_system(value: &Value, plan: bool) -> bool {
    value.as_str().is_some_and(nonempty)
        || value
            .as_array()
            .is_some_and(|items| items.iter().all(|item| valid_system_block(item, plan)))
}

fn valid_system_block(value: &Value, plan: bool) -> bool {
    let Some(block) = value.as_object() else {
        return false;
    };
    if plan {
        return valid_block(value, true, true);
    }
    fields(block, &["type", "text"])
        && block.get("type").and_then(Value::as_str) == Some("text")
        && block.get("text").is_some_and(Value::is_string)
}

fn valid_block(value: &Value, plan: bool, system: bool) -> bool {
    let Some(block) = value.as_object() else {
        return false;
    };
    let Some(kind) = block.get("type").and_then(Value::as_str) else {
        return false;
    };
    if kind == "text" {
        return fields(block, &["type", "text", "cache_control"])
            && block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| system || nonempty(text))
            && valid_marker(block);
    }
    if kind == "thinking" {
        return plan
            && !system
            && fields(block, &["type", "thinking", "signature"])
            && block.get("thinking").is_some_and(Value::is_string)
            && block
                .get("signature")
                .and_then(Value::as_str)
                .is_some_and(nonempty);
    }
    if kind == "tool_use" {
        return !system
            && fields(block, &["type", "id", "name", "input", "cache_control"])
            && block
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(nonempty)
            && block
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(nonempty)
            && block.get("input").is_some_and(Value::is_object)
            && valid_marker(block);
    }
    kind == "tool_result"
        && !system
        && fields(
            block,
            &[
                "type",
                "tool_use_id",
                "content",
                "is_error",
                "cache_control",
            ],
        )
        && block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .is_some_and(nonempty)
        && block.get("content").is_some_and(|item| {
            item.is_string()
                || item
                    .as_array()
                    .is_some_and(|items| items.iter().all(valid_result_text))
        })
        && block.get("is_error").is_none_or(Value::is_boolean)
        && valid_marker(block)
}

fn valid_result_text(value: &Value) -> bool {
    value.as_object().is_some_and(|item| {
        fields(item, &["type", "text"])
            && item.get("type").and_then(Value::as_str) == Some("text")
            && item.get("text").is_some_and(Value::is_string)
    })
}
fn valid_tool(value: &Value, plan: bool) -> bool {
    let Some(tool) = value.as_object() else {
        return false;
    };
    let allowed = if plan {
        &[
            "name",
            "description",
            "input_schema",
            "type",
            "cache_control",
        ][..]
    } else {
        &["name", "description", "input_schema", "type"][..]
    };
    fields(tool, allowed)
        && tool
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(nonempty)
        && tool.get("description").is_none_or(Value::is_string)
        && tool.get("input_schema").is_some_and(Value::is_object)
        && tool
            .get("type")
            .is_none_or(|item| item.as_str() == Some("custom"))
        && (!plan || valid_marker(tool))
}
fn fields(object: &JsonObject, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}
fn valid_marker(object: &JsonObject) -> bool {
    object
        .get("cache_control")
        .is_none_or(|item| cache_ttl(item).is_some())
}
fn cache_ttl(value: &Value) -> Option<u32> {
    let object = value.as_object()?;
    if !fields(object, &["type", "ttl"])
        || object.get("type").and_then(Value::as_str) != Some("ephemeral")
    {
        return None;
    }
    match object.get("ttl").and_then(Value::as_str).unwrap_or("5m") {
        "5m" => Some(300),
        "1h" => Some(3600),
        _ => None,
    }
}
fn nonempty(value: &str) -> bool {
    !value.is_empty() && !value.trim().is_empty()
}
fn integer(value: &Value) -> bool {
    value.as_i64().is_some() || value.as_u64().is_some()
}
fn number(value: &Value) -> bool {
    value.as_f64().is_some() && !value.is_boolean()
}

fn positions(body: &JsonObject) -> Vec<Position> {
    let mut result = Vec::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            result.push(make_position(Section::Tools, -1, "", tool));
        }
    }
    if let Some(system) = body.get("system") {
        for block in blocks(system) {
            result.push(make_position(Section::System, -1, "", block));
        }
    }
    if let Some(messages) = body.get("messages").and_then(Value::as_array) {
        for (index, message) in messages.iter().enumerate() {
            if let Some(message) = message.as_object() {
                if let Some(content) = message.get("content") {
                    for block in blocks(content) {
                        result.push(make_position(
                            Section::Messages,
                            i64::try_from(index).unwrap_or(i64::MAX),
                            message.get("role").and_then(Value::as_str).unwrap_or(""),
                            block,
                        ));
                    }
                }
            }
        }
    }
    result
}
fn blocks(value: &Value) -> Vec<&Value> {
    match value {
        Value::String(_) => vec![value],
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    }
}
fn make_position(section: Section, message_index: i64, role: &str, value: &Value) -> Position {
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
fn cacheable(position: &Position) -> bool {
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
fn selected_settings(body: &JsonObject) -> Value {
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
fn chained_hashes(positions: &[Position], body: &JsonObject, settings: &Value) -> Vec<String> {
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
fn section(value: Section) -> &'static str {
    match value {
        Section::Tools => "tools",
        Section::System => "system",
        Section::Messages => "messages",
    }
}
fn position_groups(positions: &[Position]) -> Vec<Vec<usize>> {
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
fn lookbacks(
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
fn prefix_body(body: &JsonObject, positions: &[Position], last: usize) -> JsonObject {
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
fn message(role: &str, content: Vec<Value>) -> Value {
    let mut result = Map::new();
    result.insert("role".to_owned(), Value::String(role.to_owned()));
    result.insert("content".to_owned(), Value::Array(content));
    Value::Object(result)
}
fn entry_fingerprint(fingerprint: &str, ttl: u32) -> String {
    digest(&Value::Array(vec![
        Value::String("native-cache-prefix-v2".to_owned()),
        Value::String(fingerprint.to_owned()),
        Value::Number(Number::from(ttl)),
    ]))
}
fn digest(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
