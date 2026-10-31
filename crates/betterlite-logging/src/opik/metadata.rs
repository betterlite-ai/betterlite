use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[must_use]
pub fn normalize_provider_name(provider: Option<&str>) -> Option<String> {
    let name = provider?;
    Some(
        match name {
            "vertex_ai-language-models" => "google_vertexai",
            "gemini" => "google_ai",
            "vertex_ai-anthropic_models" => "anthropic_vertexai",
            "bedrock_converse" => "bedrock",
            other => other,
        }
        .to_owned(),
    )
}

#[must_use]
pub fn extract_opik_metadata(
    litellm_metadata: &IndexMap<String, Json>,
    standard_logging_metadata: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut result = object_field(standard_logging_metadata, "user_api_key_auth_metadata")
        .and_then(|auth| object_field(auth, "opik"))
        .cloned()
        .unwrap_or_default();
    merge_nested(&mut result, litellm_metadata, "opik");
    if let Some(requester) = object_field(standard_logging_metadata, "requester_metadata") {
        merge_nested(&mut result, requester, "opik");
    }
    result
}

#[must_use]
pub fn extract_span_identifiers(
    current_span_data: Option<&Json>,
) -> (Option<String>, Option<String>) {
    let Some(Json::Object(data)) = current_span_data else {
        if current_span_data.is_some() {
            tracing::warn!("unexpected Opik current_span_data format");
        }
        return (None, None);
    };
    (owned_text(data, "trace_id"), owned_text(data, "id"))
}

#[must_use]
pub fn extract_tags(metadata: &IndexMap<String, Json>, provider: Option<&str>) -> Vec<Json> {
    let mut tags = match metadata.get("tags") {
        Some(Json::List(items)) => items.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Vec::new(),
    };
    if let Some(name) = provider.filter(|candidate| !candidate.is_empty()) {
        tags.push(Json::Text(name.to_owned()));
    }
    tags
}

#[must_use]
pub fn apply_proxy_header_overrides(
    mut project_name: String,
    mut tags: Vec<Json>,
    mut thread_id: Option<String>,
    headers: &BTreeMap<String, String>,
) -> (String, Vec<Json>, Option<String>) {
    for (key, value) in headers {
        if value.is_empty() || !key.starts_with("opik_") {
            continue;
        }
        match key.strip_prefix("opik_") {
            Some("project_name") => project_name.clone_from(value),
            Some("thread_id") => thread_id = Some(value.clone()),
            Some("tags") => {
                if let Ok(parsed) = serde_json::from_str::<Vec<Json>>(value) {
                    tags.extend(parsed);
                } else {
                    tracing::warn!("failed to parse Opik tags header");
                }
            }
            _ => {}
        }
    }
    (project_name, tags, thread_id)
}

#[must_use]
pub fn extract_and_build_metadata(
    opik_metadata: &IndexMap<String, Json>,
    standard_logging_metadata: &IndexMap<String, Json>,
    standard_logging_object: &IndexMap<String, Json>,
    litellm_kwargs: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut metadata: IndexMap<String, Json> = opik_metadata
        .iter()
        .filter(|(key, _)| key.as_str() != "current_span_data")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    metadata.insert("created_from".to_owned(), Json::Text("litellm".to_owned()));
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
        let cost_entry = IndexMap::from([
            ("total_tokens".to_owned(), cost.clone()),
            ("currency".to_owned(), Json::Text("USD".to_owned())),
        ]);
        metadata.insert("cost".to_owned(), Json::Object(cost_entry));
    }
    if let Some(debug) = litellm_kwargs.get("response_cost_failure_debug_info") {
        metadata.insert("response_cost_failure_debug_info".to_owned(), debug.clone());
    }
    metadata
}

fn merge_nested(target: &mut IndexMap<String, Json>, source: &IndexMap<String, Json>, key: &str) {
    if let Some(values) = object_field(source, key) {
        target.extend(values.clone());
    }
}

fn object_field<'source>(
    source: &'source IndexMap<String, Json>,
    key: &str,
) -> Option<&'source IndexMap<String, Json>> {
    match source.get(key)? {
        Json::Object(value) => Some(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn owned_text(source: &IndexMap<String, Json>, key: &str) -> Option<String> {
    match source.get(key)? {
        Json::Text(value) => Some(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
