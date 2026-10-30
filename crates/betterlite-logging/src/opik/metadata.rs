use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub fn normalize_provider_name(provider: Option<&str>) -> Option<String> {
    let value = provider.map(|provider| {
        match provider {
            "vertex_ai-language-models" => "google_vertexai",
            "gemini" => "google_ai",
            "vertex_ai-anthropic_models" => "anthropic_vertexai",
            "bedrock_converse" => "bedrock",
            provider => provider,
        }
        .to_owned()
    });
    value
}

pub fn extract_opik_metadata(
    litellm_metadata: &Map<String, Value>,
    standard_logging_metadata: &Map<String, Value>,
) -> Map<String, Value> {
    let mut result = standard_logging_metadata
        .get("user_api_key_auth_metadata")
        .and_then(Value::as_object)
        .and_then(|value| value.get("opik"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    merge_nested(&mut result, litellm_metadata, "opik");
    if let Some(requester) = standard_logging_metadata
        .get("requester_metadata")
        .and_then(Value::as_object)
    {
        merge_nested(&mut result, requester, "opik");
    }
    result
}

pub fn extract_span_identifiers(
    current_span_data: Option<&Value>,
) -> (Option<String>, Option<String>) {
    let Some(Value::Object(data)) = current_span_data else {
        if current_span_data.is_some() {
            tracing::warn!("unexpected Opik current_span_data format");
        }
        return (None, None);
    };
    (string_value(data, "trace_id"), string_value(data, "id"))
}

pub fn extract_tags(metadata: &Map<String, Value>, provider: Option<&str>) -> Vec<Value> {
    let mut tags = metadata
        .get("tags")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(provider) = provider.filter(|provider| !provider.is_empty()) {
        tags.push(Value::String(provider.to_owned()));
    }
    tags
}

pub fn apply_proxy_header_overrides(
    project_name: String,
    mut tags: Vec<Value>,
    thread_id: Option<String>,
    headers: &BTreeMap<String, String>,
) -> (String, Vec<Value>, Option<String>) {
    let mut project_name = project_name;
    let mut thread_id = thread_id;
    for (key, value) in headers {
        if value.is_empty() || !key.starts_with("opik_") {
            continue;
        }
        match key.strip_prefix("opik_") {
            Some("project_name") => project_name = value.clone(),
            Some("thread_id") => thread_id = Some(value.clone()),
            Some("tags") => match serde_json::from_str::<Value>(value) {
                Ok(Value::Array(parsed)) => tags.extend(parsed),
                Ok(_) | Err(_) => tracing::warn!("failed to parse Opik tags header"),
            },
            _ => {}
        }
    }
    (project_name, tags, thread_id)
}

pub fn extract_and_build_metadata(
    opik_metadata: &Map<String, Value>,
    standard_logging_metadata: &Map<String, Value>,
    standard_logging_object: &Map<String, Value>,
    litellm_kwargs: &Map<String, Value>,
) -> Map<String, Value> {
    let mut metadata: Map<String, Value> = opik_metadata
        .iter()
        .filter(|(key, _)| key.as_str() != "current_span_data")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    metadata.insert(
        "created_from".to_owned(),
        Value::String("litellm".to_owned()),
    );
    metadata.extend(standard_logging_metadata.clone());
    for (source, dest) in [
        ("call_type", "type"),
        ("status", "status"),
        ("model", "model"),
        ("model_id", "model_id"),
        ("model_group", "model_group"),
        ("api_base", "api_base"),
        ("cache_hit", "cache_hit"),
        ("saved_cache_cost", "saved_cache_cost"),
        ("error_str", "error_str"),
        ("model_parameters", "model_parameters"),
        ("hidden_params", "hidden_params"),
        ("model_map_information", "model_map_information"),
    ] {
        if let Some(value) = standard_logging_object.get(source) {
            metadata.insert(dest.to_owned(), value.clone());
        }
    }
    if let Some(cost) = litellm_kwargs.get("response_cost") {
        metadata.insert(
            "cost".to_owned(),
            serde_json::json!({"total_tokens": cost, "currency": "USD"}),
        );
    }
    if let Some(debug) = litellm_kwargs.get("response_cost_failure_debug_info") {
        metadata.insert("response_cost_failure_debug_info".to_owned(), debug.clone());
    }
    metadata
}

fn merge_nested(target: &mut Map<String, Value>, source: &Map<String, Value>, key: &str) {
    if let Some(values) = source.get(key).and_then(Value::as_object) {
        target.extend(values.clone());
    }
}

fn string_value(source: &Map<String, Value>, key: &str) -> Option<String> {
    let value = source.get(key).and_then(Value::as_str).map(str::to_owned);
    value
}
