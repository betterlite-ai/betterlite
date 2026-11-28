use std::num::ParseIntError;

use serde_json::Number;
use thiserror::Error;

use crate::call_helpers::drop_params::normalize_drop_params;
use crate::decisions::Json;
use crate::headers::affinity::{AffinityError, validate_affinity_header};
use crate::model_registration::JsonObject;

// keys that would collide with Python's own argument names when GenericLiteLLMParams unpacks the dict
const RESERVED_KEYS: [&str; 3] = ["self", "params", "__class__"];

// the five GenericLiteLLMParams fields whose default is False, so exclude_none keeps them when the caller leaves them out
const FALSE_DEFAULTS: [&str; 5] = [
    "allow_client_keepalive_override",
    "use_in_pass_through",
    "use_litellm_proxy",
    "use_xai_oauth",
    "merge_reasoning_content_in_choices",
];

#[derive(Debug, Error)]
pub enum GenericParamsError {
    #[error("max_retries {value:?} is not an integer")]
    MaxRetries {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error(transparent)]
    Affinity(#[from] AffinityError),
}

// GenericLiteLLMParams(**raw).model_dump(exclude_none=True): reserved keys gone, max_retries and drop_params normalised, affinity checked, nulls left out, the five False defaults filled in
pub fn normalize_generic_params(raw: &JsonObject) -> Result<JsonObject, GenericParamsError> {
    let mut params: JsonObject = raw
        .iter()
        .filter(|(key, _)| !RESERVED_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(Json::Text(text)) = params.get("max_retries") {
        let count =
            text.trim()
                .parse::<i64>()
                .map_err(|source| GenericParamsError::MaxRetries {
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
    // a flag the input left out takes its False default; one the input set to null stays out
    for flag in FALSE_DEFAULTS {
        if !raw.contains_key(flag) {
            params.insert(flag.to_owned(), Json::Bool(false));
        }
    }
    params.retain(|_, value| !matches!(value, Json::Null));
    Ok(params)
}
