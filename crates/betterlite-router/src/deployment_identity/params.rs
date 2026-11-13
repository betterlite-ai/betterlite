use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::validate_affinity_header;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::error::InvalidDeployment;

// Keys that would collide with Python's own argument names when LiteLLM unpacks the dict into a call.
const RESERVED_KEYS: [&str; 3] = ["self", "params", "__class__"];
// Flags that LiteLLM_Params defaults to false and so always carries in its dump.
const FALSE_DEFAULTS: [&str; 5] = [
    "allow_client_keepalive_override",
    "use_in_pass_through",
    "use_litellm_proxy",
    "use_xai_oauth",
    "merge_reasoning_content_in_choices",
];

// LiteLLM_Params(**raw) as its dump reads: reserved keys gone, max_retries and drop_params normalised, affinity header checked, nulls left out.
pub(super) fn litellm_params(raw: &JsonObject) -> Result<JsonObject, InvalidDeployment> {
    let mut params: JsonObject = raw
        .iter()
        .filter(|(key, _)| !RESERVED_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !matches!(params.get("model"), Some(Json::Text(_))) {
        return Err(InvalidDeployment::MissingModel);
    }
    if let Some(Json::Text(text)) = params.get("max_retries") {
        let count = text
            .trim()
            .parse::<i64>()
            .map_err(|source| InvalidDeployment::MaxRetries {
                value: text.clone(),
                source,
            })?;
        params.insert("max_retries".to_owned(), Json::Number(Number::from(count)));
    }
    if let Some(Json::Text(header)) = params.get("provider_affinity_header") {
        validate_affinity_header(header)?;
    }
    if let Some(given) = params.get("drop_params") {
        match (normalize_drop_params(Some(given)), given) {
            (Some(flag), _) => {
                params.insert("drop_params".to_owned(), Json::Bool(flag));
            }
            (None, Json::Text(_) | Json::Null) => {}
            (None, other) => {
                tracing::warn!("drop_params={other:?} is not a flag value, treating it as unset");
                params.shift_remove("drop_params");
            }
        }
    }
    for flag in FALSE_DEFAULTS {
        params.entry(flag.to_owned()).or_insert(Json::Bool(false));
    }
    params.retain(|_, value| !matches!(value, Json::Null));
    Ok(params)
}
