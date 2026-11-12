use std::collections::BTreeSet;

use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;

use super::Params;
use super::names::{
    DEFAULT_CHAT_COMPLETION_PARAMS, OPENAI_CHAT_COMPLETION_PARAMS, OPENAI_TRANSCRIPTION_PARAMS,
    PROVIDER_UNVALIDATED_PARAMS,
};
use crate::owned_parameter_names::is_litellm_owned_kwarg;

#[must_use]
pub fn select_non_default_params(
    passed_params: &Params,
    default_names: &[&str],
    additional_drop_params: &[String],
) -> Params {
    let mut selected = passed_params.clone();
    selected.retain(|key, value| {
        default_names.contains(&key.as_str())
            && !matches!(value, JsonValue::Null)
            && !additional_drop_params.contains(key)
    });
    selected
}

pub fn bedrock_route_for_request<R, E, F>(
    model: &str,
    passed_params: &Params,
    additional_drop_params: &[String],
    route: F,
) -> Result<R, E>
where
    F: FnOnce(&str, &Params) -> Result<R, E>,
{
    let mut filtered = passed_params.clone();
    filtered.retain(|key, _| !additional_drop_params.contains(key));
    route(model, &filtered)
}

#[must_use]
pub fn get_standard_openai_params(params: &Params) -> Params {
    params
        .iter()
        .filter(|(key, value)| {
            OPENAI_CHAT_COMPLETION_PARAMS
                .split_whitespace()
                .any(|name| name == key.as_str())
                && !matches!(value, JsonValue::Null)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[must_use]
pub fn get_non_default_params(passed_params: &Params) -> Params {
    let names: Vec<&str> = DEFAULT_CHAT_COMPLETION_PARAMS.split_whitespace().collect();
    let mut selected = select_non_default_params(passed_params, &names, &[]);
    selected.retain(|key, _| !matches!(key.as_str(), "model" | "custom_llm_provider"));
    selected
}

#[must_use]
pub fn provider_rejectable_params(passed_params: &Params) -> BTreeSet<String> {
    get_non_default_params(passed_params)
        .into_keys()
        .filter(|key| !PROVIDER_UNVALIDATED_PARAMS.contains(&key.as_str()))
        .collect()
}

#[must_use]
pub fn filter_out_litellm_params(kwargs: &Params, excluding: &[&str]) -> Params {
    let mut kept = kwargs.clone();
    kept.retain(|key, _| !excluding.contains(&key.as_str()) && !is_litellm_owned_kwarg(key));
    kept
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    Completion,
    Transcription,
}

#[must_use]
pub fn non_default_kwargs(kwargs: &Params, endpoint: Endpoint) -> Params {
    let chat_names: Vec<&str> = OPENAI_CHAT_COMPLETION_PARAMS.split_whitespace().collect();
    match endpoint {
        Endpoint::Completion => filter_out_litellm_params(kwargs, &chat_names),
        Endpoint::Transcription => filter_out_litellm_params(kwargs, OPENAI_TRANSCRIPTION_PARAMS),
    }
}

#[must_use]
pub fn remove_unsupported_params(
    mut non_default_params: Params,
    supported_openai_params: Option<&[String]>,
) -> Params {
    let Some(supported) = supported_openai_params else {
        return Params::new();
    };
    non_default_params.retain(|key, _| supported.contains(key));
    non_default_params
}

#[must_use]
pub fn apply_openai_param_overrides(
    mut optional_params: Params,
    non_default_params: &mut Params,
    allowed_openai_params: &[String],
) -> Params {
    for param in allowed_openai_params {
        if optional_params.contains_key(param) {
            continue;
        }
        if let Some(value) = non_default_params.remove(param) {
            optional_params.insert(param.clone(), value);
        }
    }
    optional_params
}
