use std::collections::{BTreeMap, BTreeSet};
use std::num::FpCategory;

use serde_json::{Map, Number, Value};
use uuid::Uuid;

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::{RouterView, StreamOptionsCheck};
use crate::pre_call::route_type::RouteType;

// LiteLLM's MAX_LITELLM_CALL_ID_LENGTH
const MAX_CALL_ID_LENGTH: usize = 256;
const STRIP_USAGE_KEY: &str = "_litellm_strip_stream_usage";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestModel {
    One(String),
    Many(Vec<String>),
    Absent,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CliOverrides {
    pub temperature: Option<f64>,
    pub request_timeout: Option<f64>,
    pub max_tokens: Option<i64>,
    pub api_base: Option<String>,
}

#[must_use]
pub fn resolve_call_id(header: Option<&str>) -> String {
    match header {
        Some(value) if (1..=MAX_CALL_ID_LENGTH).contains(&value.chars().count()) => {
            value.to_owned()
        }
        Some(_) | None => Uuid::new_v4().to_string(),
    }
}

// the support check is the last gate and runs only for a chat stream, since it costs a router lookup
pub fn stream_usage_updates<F>(
    data: &Map<String, Value>,
    always_include: Option<bool>,
    route_type: RouteType,
    supports: F,
) -> Map<String, Value>
where
    F: FnOnce() -> bool,
{
    let mut scrub = Map::new();
    if data.contains_key(STRIP_USAGE_KEY) {
        scrub.insert(STRIP_USAGE_KEY.to_owned(), Value::Bool(false));
    }
    if data.get("stream") != Some(&Value::Bool(true)) {
        return scrub;
    }
    let stream_options = data.get("stream_options");
    if always_include == Some(true) {
        return forced_usage(scrub, stream_options);
    }
    if always_include == Some(false) || route_type != RouteType::Completion {
        return scrub;
    }
    let existing = stream_options.and_then(Value::as_object);
    if existing.and_then(|options| options.get("include_usage")) == Some(&Value::Bool(true))
        || !supports()
    {
        return scrub;
    }
    let mut merged = existing.cloned().unwrap_or_default();
    merged.insert("include_usage".to_owned(), Value::Bool(true));
    let mut updates = Map::new();
    updates.insert("stream_options".to_owned(), Value::Object(merged));
    updates.insert(STRIP_USAGE_KEY.to_owned(), Value::Bool(true));
    updates
}

fn forced_usage(
    mut updates: Map<String, Value>,
    stream_options: Option<&Value>,
) -> Map<String, Value> {
    match stream_options {
        None => {
            let mut options = Map::new();
            options.insert("include_usage".to_owned(), Value::Bool(true));
            updates.insert("stream_options".to_owned(), Value::Object(options));
        }
        Some(Value::Object(existing)) if !existing.contains_key("include_usage") => {
            let mut merged = existing.clone();
            merged.insert("include_usage".to_owned(), Value::Bool(true));
            updates.insert("stream_options".to_owned(), Value::Object(merged));
        }
        Some(_) => {}
    }
    updates
}

pub fn streamable_models<K>(
    model: &Value,
    router: Option<&dyn RouterView<K>>,
    team_id: Option<&str>,
    check: &dyn StreamOptionsCheck,
) -> bool
where
    K: PreCallKey,
{
    let Some(name) = model.as_str() else {
        return false;
    };
    let resolved = router.map_or_else(Vec::new, |view| view.resolved_models(name, team_id));
    if resolved.is_empty() {
        return check.supports(name);
    }
    resolved.iter().all(|candidate| check.supports(candidate))
}

pub fn file_search_store_ids(
    data: &Map<String, Value>,
) -> Result<BTreeSet<String>, PreCallFailure> {
    let mut store_ids = BTreeSet::new();
    let Some(Value::Array(tools)) = data.get("tools") else {
        return Ok(store_ids);
    };
    for tool in tools {
        let Value::Object(fields) = tool else {
            continue;
        };
        if fields.get("type").and_then(Value::as_str) != Some("file_search") {
            continue;
        }
        for id in tool_store_ids(fields)? {
            match id.as_str() {
                Some(text) if !text.is_empty() => store_ids.insert(text.to_owned()),
                Some(_) | None => return Err(bad_store_ids()),
            };
        }
    }
    Ok(store_ids)
}

// `vector_store_ids or []`: an absent, null, false or zero value is an empty list
fn tool_store_ids(fields: &Map<String, Value>) -> Result<&[Value], PreCallFailure> {
    match fields.get("vector_store_ids") {
        None | Some(Value::Null | Value::Bool(false)) => Ok(&[]),
        Some(Value::Number(number))
            if number
                .as_f64()
                .is_some_and(|value| value.classify() == FpCategory::Zero) =>
        {
            Ok(&[])
        }
        Some(Value::Array(ids)) => Ok(ids.as_slice()),
        Some(_) => Err(bad_store_ids()),
    }
}

fn bad_store_ids() -> PreCallFailure {
    let mut detail = Map::new();
    detail.insert(
        "error".to_owned(),
        Value::String("file_search.vector_store_ids must be a list of strings".to_owned()),
    );
    PreCallFailure::Http {
        status: 400,
        detail: Value::Object(detail),
    }
}

#[must_use]
pub fn request_model(data: &Map<String, Value>) -> RequestModel {
    match data.get("model") {
        Some(Value::String(model)) => RequestModel::One(model.clone()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<String>>>()
            .map_or(RequestModel::Absent, RequestModel::Many),
        _ => RequestModel::Absent,
    }
}

// Python's `if user_temperature:` skips 0, 0.0 and "" alike
pub fn apply_overrides(data: &mut Map<String, Value>, overrides: &CliOverrides) {
    let numbers = [
        ("temperature", overrides.temperature),
        ("request_timeout", overrides.request_timeout),
    ];
    for (name, value) in numbers {
        let setting = value.filter(|setting| setting.classify() != FpCategory::Zero);
        if let Some(number) = setting.and_then(Number::from_f64) {
            data.insert(name.to_owned(), Value::Number(number));
        }
    }
    if let Some(tokens) = overrides.max_tokens.filter(|tokens| *tokens != 0) {
        data.insert("max_tokens".to_owned(), Value::from(tokens));
    }
    if let Some(base) = overrides.api_base.as_ref().filter(|base| !base.is_empty()) {
        data.insert("api_base".to_owned(), Value::String(base.clone()));
    }
}

pub fn apply_aliases<K>(
    data: &mut Map<String, Value>,
    alias_map: &BTreeMap<String, String>,
    key: &K,
) where
    K: PreCallKey,
{
    let configured = data
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| alias_map.get(model));
    if let Some(target) = configured {
        data.insert("model".to_owned(), Value::String(target.clone()));
    }
    let keyed = data
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| key.key_aliases()?.get(model));
    if let Some(target) = keyed {
        data.insert("model".to_owned(), target.clone());
    }
}

pub fn write_queue_time(data: &mut Map<String, Value>, bucket: &str, start: f64) {
    let arrival = data
        .get("proxy_server_request")
        .and_then(|request| request.get("arrival_time"))
        .and_then(Value::as_f64);
    #[expect(
        clippy::float_arithmetic,
        reason = "LiteLLM's queue time is the f64 timestamp difference start - arrival_time"
    )]
    let difference = arrival.and_then(|arrival_time| Number::from_f64(start - arrival_time));
    let Some(queue_time) = difference else {
        return;
    };
    let entry = data
        .entry(bucket.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Value::Object(fields) = entry {
        fields.insert("queue_time_seconds".to_owned(), Value::Number(queue_time));
    }
}
