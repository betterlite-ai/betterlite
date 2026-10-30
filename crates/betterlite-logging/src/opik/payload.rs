use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use super::{
    apply_proxy_header_overrides, create_usage_object, create_uuid7, extract_and_build_metadata,
    extract_opik_metadata, extract_span_identifiers, extract_tags, normalize_provider_name,
};

pub fn build_trace_payload(
    project_name: &str,
    trace_id: &str,
    response: &Map<String, Value>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    input: Value,
    output: Value,
    metadata: Map<String, Value>,
    tags: Vec<Value>,
    thread_id: Option<String>,
) -> Map<String, Value> {
    let mut payload = common_payload(project_name, start, end, input, output, metadata, tags);
    payload.extend([
        ("id".to_owned(), Value::String(trace_id.to_owned())),
        (
            "name".to_owned(),
            Value::String(string_or(response, "object", "unknown type")),
        ),
    ]);
    if let Some(thread_id) = thread_id {
        payload.insert("thread_id".to_owned(), Value::String(thread_id));
    }
    remove_nulls(payload)
}

pub fn build_span_payload(
    project_name: &str,
    trace_id: &str,
    parent_span_id: Option<String>,
    response: &Map<String, Value>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    input: Value,
    output: Value,
    metadata: Map<String, Value>,
    tags: Vec<Value>,
    cost: Option<Value>,
    provider: Option<&str>,
) -> Map<String, Value> {
    let model = string_or(response, "model", "unknown-model");
    let name = format!(
        "{}_{}_{}",
        model,
        string_or(response, "object", "unknown-object"),
        integer_or(response, "created", 0)
    );
    let usage = response
        .get("usage")
        .and_then(Value::as_object)
        .map_or_else(Map::new, create_usage_object);
    let mut payload = common_payload(project_name, start, end, input, output, metadata, tags);
    payload.extend([
        ("id".to_owned(), Value::String(create_uuid7())),
        ("trace_id".to_owned(), Value::String(trace_id.to_owned())),
        ("name".to_owned(), Value::String(name)),
        ("type".to_owned(), Value::String("llm".to_owned())),
        ("model".to_owned(), Value::String(model)),
        (
            "start_time".to_owned(),
            Value::String(start.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        ),
        (
            "end_time".to_owned(),
            Value::String(end.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        ),
        (
            "parent_span_id".to_owned(),
            parent_span_id.map_or(Value::Null, Value::String),
        ),
        (
            "provider".to_owned(),
            provider.map_or(Value::Null, |value| Value::String(value.to_owned())),
        ),
        ("total_cost".to_owned(), cost.unwrap_or(Value::Null)),
        ("usage".to_owned(), Value::Object(usage)),
    ]);
    remove_nulls(payload)
}

pub fn build_opik_payload(
    kwargs: &Map<String, Value>,
    response: &Map<String, Value>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    project_name: &str,
) -> (Option<Map<String, Value>>, Map<String, Value>) {
    let empty = Map::new();
    let standard = kwargs
        .get("standard_logging_object")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let litellm_params = kwargs
        .get("litellm_params")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let litellm_metadata = litellm_params
        .get("metadata")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let standard_metadata = standard
        .get("metadata")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let opik_metadata = extract_opik_metadata(litellm_metadata, standard_metadata);
    let current_span_data = opik_metadata.get("current_span_data");
    let (existing_trace_id, parent_span_id) = extract_span_identifiers(current_span_data);
    let trace_id = existing_trace_id.clone().unwrap_or_else(create_uuid7);
    let mut current_project = opik_metadata
        .get("project_name")
        .and_then(Value::as_str)
        .unwrap_or(project_name)
        .to_owned();
    let provider_raw = kwargs.get("custom_llm_provider").and_then(Value::as_str);
    let mut tags = extract_tags(&opik_metadata, provider_raw);
    let mut thread_id = opik_metadata
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let headers = litellm_params
        .get("proxy_server_request")
        .and_then(Value::as_object)
        .and_then(|request| request.get("headers"))
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    (current_project, tags, thread_id) =
        apply_proxy_header_overrides(current_project, tags, thread_id, &headers);
    let metadata = extract_and_build_metadata(&opik_metadata, standard_metadata, standard, kwargs);
    let input = standard
        .get("messages")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let output = standard
        .get("response")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let trace = if existing_trace_id.is_none() {
        Some(build_trace_payload(
            &current_project,
            &trace_id,
            response,
            start,
            end,
            input.clone(),
            output.clone(),
            metadata.clone(),
            tags.clone(),
            thread_id,
        ))
    } else {
        None
    };
    let provider = normalize_provider_name(provider_raw);
    let span = build_span_payload(
        &current_project,
        &trace_id,
        parent_span_id,
        response,
        start,
        end,
        input,
        output,
        metadata,
        tags,
        kwargs.get("response_cost").cloned(),
        provider.as_deref(),
    );
    (trace, span)
}

pub fn classify_payload(payload: &[Value]) -> (Vec<Map<String, Value>>, Vec<Map<String, Value>>) {
    let traces = payload
        .iter()
        .filter_map(Value::as_object)
        .filter(|item| !item.contains_key("type"))
        .cloned()
        .map(remove_nulls)
        .collect();
    let spans = payload
        .iter()
        .filter_map(Value::as_object)
        .filter(|item| item.contains_key("type"))
        .cloned()
        .map(remove_nulls)
        .collect();
    let result = (traces, spans);
    result
}

fn common_payload(
    project: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    input: Value,
    output: Value,
    metadata: Map<String, Value>,
    tags: Vec<Value>,
) -> Map<String, Value> {
    let payload = BTreeMap::from([
        ("project_name".to_owned(), Value::String(project.to_owned())),
        (
            "start_time".to_owned(),
            Value::String(start.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        ),
        (
            "end_time".to_owned(),
            Value::String(end.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        ),
        ("input".to_owned(), input),
        ("output".to_owned(), output),
        ("metadata".to_owned(), Value::Object(metadata)),
        ("tags".to_owned(), Value::Array(tags)),
    ]);
    payload.into_iter().collect()
}

fn remove_nulls(mut payload: Map<String, Value>) -> Map<String, Value> {
    payload.retain(|_, value| !value.is_null());
    payload
}
fn string_or(source: &Map<String, Value>, key: &str, default: &str) -> String {
    let value = source
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned();
    value
}
fn integer_or(source: &Map<String, Value>, key: &str, default: i64) -> i64 {
    let value = source.get(key).and_then(Value::as_i64).unwrap_or(default);
    value
}
