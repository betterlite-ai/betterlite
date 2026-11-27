use std::time::Duration;

use betterlite_context::core_settings::CoreSettings;
use betterlite_context::resource_config_selection::skills_evals::select_evals_config;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use crate::api::eval_create_list::call::EvalsCallParams;
use crate::api::eval_create_list::config::EvalsConfigLookup;
use crate::api::eval_create_list::{DEFAULT_EVALS_PROVIDER, DEFAULT_OPENAI_API_BASE};
use crate::api::eval_http::exchange::EvalConfig;

use super::error::EvalsGetUpdateError;
use super::request::decode;
use super::service::Operation;

// what both operations resolve before they build their request: the provider, its config and validated headers
pub(super) struct ProviderCall<'deps> {
    pub(super) provider: String,
    pub(super) config: EvalConfig<'deps>,
    pub(super) headers: Headers,
    pub(super) litellm_params: Object,
    pub(super) timeout: Duration,
}

// LiteLLM's provider setup: the default provider, its evals config, the validated headers and the resolved timeout
pub(super) fn prepare<'deps, Configs>(
    configs: &'deps Configs,
    core: &CoreSettings,
    params: &EvalsCallParams<'_>,
    operation: Operation,
) -> Result<ProviderCall<'deps>, EvalsGetUpdateError>
where
    Configs: EvalsConfigLookup,
{
    let provider = params.custom_llm_provider.unwrap_or(DEFAULT_EVALS_PROVIDER);
    let kind =
        provider
            .parse::<ProviderId>()
            .map_err(|_source| EvalsGetUpdateError::UnknownProvider {
                provider: provider.to_owned(),
            })?;
    let tag = select_evals_config(kind).ok_or_else(|| operation.unsupported(provider))?;
    let config = configs
        .evals_config(tag)
        .ok_or_else(|| operation.unsupported(provider))?;
    let litellm_params = litellm_params_object(params.kwargs)?;
    let extra_headers = params.extra_headers.cloned().unwrap_or_default();
    let headers = config.validate_environment(extra_headers, Some(&litellm_params))?;
    // LiteLLM's `timeout or request_timeout`: an absent or zero timeout takes REQUEST_TIMEOUT
    let timeout = match params.timeout {
        Some(value) if !value.is_zero() => value,
        Some(_) | None => {
            Duration::try_from_secs_f64(core.request_timeout_seconds).map_err(|source| {
                EvalsGetUpdateError::Timeout {
                    seconds: core.request_timeout_seconds,
                    source,
                }
            })?
        }
    };
    Ok(ProviderCall {
        provider: provider.to_owned(),
        config,
        headers,
        litellm_params,
        timeout,
    })
}

// litellm_params.api_base, absent or null taking DEFAULT_OPENAI_API_BASE and any text used as given
pub(super) fn api_base(kwargs: &IndexMap<String, Json>) -> Result<String, EvalsGetUpdateError> {
    let Some(value) = kwargs.get("api_base") else {
        return Ok(DEFAULT_OPENAI_API_BASE.to_owned());
    };
    Ok(decode("api_base", value)?.unwrap_or_else(|| DEFAULT_OPENAI_API_BASE.to_owned()))
}

// GenericLiteLLMParams(**kwargs): the provider transforms read the caller's keywords as one object
fn litellm_params_object(kwargs: &IndexMap<String, Json>) -> Result<Object, EvalsGetUpdateError> {
    serde_json::to_value(kwargs)
        .and_then(serde_json::from_value::<Object>)
        .map_err(EvalsGetUpdateError::from)
}
