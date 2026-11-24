use std::error::Error;

use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_openai::credentials::{DataResidency, infer_openai_data_residency};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::{NativeWsCall, NativeWsCallError, NativeWsDeps};
use crate::api::responses_provider_selection::prefixes::strip_responses_routing_prefix;
use crate::api::responses_provider_selection::provider_config::{
    ConfigLookup, ResponsesApiConfig, resolve_provider_config,
};
use crate::api::responses_stream_events::JsonShape;
use crate::api::responses_ws_native::logging::WsCallUpdate;
use crate::api::responses_ws_native::request::is_supplied;

pub(super) struct ResolvedCall {
    pub(super) model: String,
    pub(super) provider: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) provider_config: Option<ResponsesApiConfig>,
}

pub(super) fn resolve_call<Client, Handler, HandlerError>(
    deps: &NativeWsDeps<'_, Handler>,
    call: &NativeWsCall<'_, Client>,
) -> Result<ResolvedCall, NativeWsCallError<HandlerError>>
where
    HandlerError: Error + 'static,
{
    let resolved = deps.resolution.resolve(&ResolutionRequest {
        model: call.model,
        custom_llm_provider: None,
        api_base: call.api_base,
        api_key: call.api_key,
    })?;
    let model = strip_responses_routing_prefix(&resolved.model);
    let provider = resolved.provider.as_deref();
    let api_base = first_text([
        resolved.api_base.as_deref(),
        call.kwargs
            .get("api_base")
            .and_then(JsonShape::optional_str),
        deps.globals.api_base.as_deref(),
    ]);
    let residency = infer_openai_data_residency(provider, api_base.as_deref());
    let litellm_params = litellm_params_for_ws(call.kwargs, residency);
    record_update(deps, call, &model, provider, &litellm_params)?;
    let kwargs_key = kwarg_secret(call.kwargs);
    let api_key = first_secret([
        resolved.api_key.as_ref(),
        kwargs_key.as_ref(),
        deps.globals.api_key.as_ref(),
        deps.globals.openai_key.as_ref(),
        deps.openai_api_key,
    ]);
    let provider_config = provider_config_for(deps.configs, provider, &model, api_base.as_deref());
    Ok(ResolvedCall {
        model,
        provider: resolved.provider,
        api_base,
        api_key,
        provider_config,
    })
}

fn provider_config_for(
    configs: &(dyn ConfigLookup + Sync),
    provider: Option<&str>,
    model: &str,
    api_base: Option<&str>,
) -> Option<ResponsesApiConfig> {
    if let Some(name) = provider {
        return resolve_provider_config(configs, model, name, &JsonValue::Null, api_base);
    }
    None
}

fn first_text<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}

fn record_update<Client, Handler>(
    deps: &NativeWsDeps<'_, Handler>,
    call: &NativeWsCall<'_, Client>,
    model: &str,
    provider: Option<&str>,
    litellm_params: &IndexMap<String, Json>,
) -> Result<(), LoggingStateError> {
    deps.logging.update_from_kwargs(WsCallUpdate {
        kwargs: call.kwargs,
        model,
        user: call.kwargs.get("user").and_then(JsonShape::optional_str),
        litellm_params,
        custom_llm_provider: provider,
    })
}

#[cfg_attr(test, mutants::skip)]
fn kwarg_secret(kwargs: &IndexMap<String, Json>) -> Option<SecretString> {
    match kwargs.get("api_key") {
        Some(Json::Text(text)) => Some(SecretString::from(text.clone())),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn litellm_params_for_ws(
    kwargs: &IndexMap<String, Json>,
    residency: Option<DataResidency>,
) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(String::from("metadata"), resolved_metadata(kwargs));
    let litellm_metadata = kwargs
        .get("litellm_metadata")
        .cloned()
        .unwrap_or(Json::Null);
    params.insert(String::from("litellm_metadata"), litellm_metadata);
    params.insert(String::from("data_residency"), residency_json(residency));
    params
}

fn resolved_metadata(kwargs: &IndexMap<String, Json>) -> Json {
    if let Some(value) = kwargs.get("metadata").filter(|value| is_supplied(value)) {
        return value.clone();
    }
    if let Some(litellm @ Json::Object(_)) = kwargs.get("litellm_metadata")
        && is_supplied(litellm)
    {
        return litellm.clone();
    }
    kwargs.get("metadata").cloned().unwrap_or(Json::Null)
}

fn residency_json(residency: Option<DataResidency>) -> Json {
    match residency {
        Some(DataResidency::Eu) => Json::Text(String::from("eu")),
        Some(DataResidency::Us) => Json::Text(String::from("us")),
        None => Json::Null,
    }
}
