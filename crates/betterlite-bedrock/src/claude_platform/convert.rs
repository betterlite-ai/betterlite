use betterlite_config::sections::claude_platform_inputs::{PlatformValue, UnsupportedParams};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::claude_platform_inputs::OVERRIDE_PARAMS_KEY;
use crate::claude_platform_inputs::body::{
    PlatformWarning, UnsupportedResolution, filter_request_body, resolve_unsupported_override,
};
use crate::model_routes::{ParamMap, ParamValue};

// The B2211 policy reads the config section's records, so each request map crosses over once, in order.
pub fn param_map(params: &IndexMap<String, Json>) -> ParamMap {
    let mut mapped = ParamMap::with_capacity(params.len());
    for (key, value) in params {
        mapped.insert(key.clone(), param_value(value));
    }
    mapped
}

pub fn body_params(body: &ParamMap) -> IndexMap<String, Json> {
    let mut mapped = IndexMap::with_capacity(body.len());
    for (key, value) in body {
        mapped.insert(key.clone(), json_value(value));
    }
    mapped
}

fn param_value(value: &Json) -> ParamValue {
    match value {
        Json::Null => ParamValue::Null,
        Json::Bool(flag) => ParamValue::Bool(*flag),
        Json::Number(number) => ParamValue::Number(number.clone()),
        Json::Text(text) => ParamValue::Text(text.clone()),
        Json::List(items) => ParamValue::List(items.iter().map(param_value).collect()),
        Json::Object(fields) => ParamValue::Mapping(param_map(fields)),
    }
}

fn json_value(value: &ParamValue) -> Json {
    match value {
        ParamValue::Null => Json::Null,
        ParamValue::Bool(flag) => Json::Bool(*flag),
        ParamValue::Number(number) => Json::Number(number.clone()),
        ParamValue::Text(text) => Json::Text(text.clone()),
        ParamValue::List(items) => Json::List(items.iter().map(json_value).collect()),
        ParamValue::Mapping(fields) => Json::Object(body_params(fields)),
    }
}

// serde already knows both shapes, so the decisions-to-prompts direction is its own round trip.
fn prompt_value(value: &Json) -> Result<PromptJson, serde_json::Error> {
    serde_json::to_string(value).and_then(|encoded| serde_json::from_str(&encoded))
}

pub fn prompt_messages(messages: &[Json]) -> Result<Vec<PromptJson>, serde_json::Error> {
    let mut mapped = Vec::with_capacity(messages.len());
    for message in messages {
        mapped.push(prompt_value(message)?);
    }
    Ok(mapped)
}

pub fn prompt_params(
    params: &IndexMap<String, Json>,
) -> Result<IndexMap<String, PromptJson>, serde_json::Error> {
    let mut mapped = IndexMap::with_capacity(params.len());
    for (key, value) in params {
        mapped.insert(key.clone(), prompt_value(value)?);
    }
    Ok(mapped)
}

pub fn contract_params(params: &IndexMap<String, PromptJson>) -> IndexMap<String, Json> {
    let mut mapped = IndexMap::with_capacity(params.len());
    for (key, value) in params {
        mapped.insert(key.clone(), Json::from(value));
    }
    mapped
}

// LiteLLM accepts a list, set, frozenset or tuple of names and warns about every other JSON shape.
fn unsupported(value: &Json) -> UnsupportedParams {
    match value {
        Json::List(items) => UnsupportedParams::List(items.iter().map(platform_value).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            UnsupportedParams::Other(platform_value(value))
        }
    }
}

fn platform_value(value: &Json) -> PlatformValue {
    match value {
        Json::Null => PlatformValue::Null,
        Json::Bool(flag) => PlatformValue::Boolean(*flag),
        Json::Number(number) => PlatformValue::Number(number.clone()),
        Json::Text(text) => PlatformValue::Text(text.clone()),
        Json::List(items) => PlatformValue::List(items.iter().map(platform_value).collect()),
        Json::Object(fields) => PlatformValue::Mapping(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), platform_value(item)))
                .collect(),
        ),
    }
}

// The request's own override wins; a null in it reads as absent and exposes the litellm_params value.
pub fn resolve_override(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    log_invalid: bool,
) -> UnsupportedResolution {
    let optional = optional_params.get(OVERRIDE_PARAMS_KEY).map(unsupported);
    let deployment = litellm_params.get(OVERRIDE_PARAMS_KEY).map(unsupported);
    resolve_unsupported_override(deployment.as_ref(), optional.as_ref(), log_invalid)
}

// Every warning the policy emitted, in the order LiteLLM logs them: the override first, then the drop.
pub fn filtered_params(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
    log_invalid: bool,
    log_dropped: bool,
) -> (ParamMap, Vec<PlatformWarning>) {
    let resolution = resolve_override(optional_params, litellm_params, log_invalid);
    let filtered = filter_request_body(
        &param_map(optional_params),
        resolution.names.as_ref(),
        log_dropped,
    );
    let mut warnings = Vec::new();
    warnings.extend(resolution.warning);
    warnings.extend(filtered.warning);
    (filtered.body, warnings)
}
