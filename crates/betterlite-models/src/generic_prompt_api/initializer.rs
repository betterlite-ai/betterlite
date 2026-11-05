use std::time::Duration;

use thiserror::Error;

use crate::decisions::Json;
use crate::generic_prompt_api::manager::{GenericPromptManager, GenericPromptSettings};
use crate::generic_prompt_api::request::PromptApiTransport;
use crate::prompt_contracts::PromptIgnoreFlags;
use crate::prompt_spec::{PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations};

pub const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GenericPromptInitializerError {
    #[error("api_base is required in generic_prompt_config")]
    MissingApiBase,
    #[error("litellm_params.timeout must be a whole number of seconds")]
    TimeoutType,
}

pub type InitializerFn<Transport> =
    fn(
        &PromptLiteLLMParams,
        &PromptSpec,
        Transport,
    ) -> Result<GenericPromptManager<Transport>, GenericPromptInitializerError>;

pub fn prompt_initializer<Transport>(
    litellm_params: &PromptLiteLLMParams,
    _prompt_spec: &PromptSpec,
    transport: Transport,
) -> Result<GenericPromptManager<Transport>, GenericPromptInitializerError>
where
    Transport: PromptApiTransport,
{
    let api_base = litellm_params
        .api_base
        .clone()
        .filter(|base| !base.is_empty())
        .ok_or(GenericPromptInitializerError::MissingApiBase)?;
    let timeout_seconds = match litellm_params.extra.get("timeout") {
        None | Some(Json::Null) => DEFAULT_TIMEOUT_SECONDS,
        Some(Json::Number(number)) => number
            .as_u64()
            .ok_or(GenericPromptInitializerError::TimeoutType)?,
        Some(_) => return Err(GenericPromptInitializerError::TimeoutType),
    };
    let settings = GenericPromptSettings {
        api_base,
        api_key: litellm_params.api_key.clone(),
        timeout: Duration::from_secs(timeout_seconds),
        prompt_id: litellm_params.prompt_id.clone(),
        provider_specific_query_params: litellm_params.provider_specific_query_params.clone(),
        ignore_flags: PromptIgnoreFlags {
            model: litellm_params.ignore_model,
            optional_params: litellm_params.ignore_optional_params,
        },
    };
    Ok(GenericPromptManager::new(settings, transport))
}

// Every variant is matched so a new integration fails to compile until wired up.
#[must_use]
pub fn prompt_initializer_registry<Transport>(
    integration: &SupportedPromptIntegrations,
) -> Option<InitializerFn<Transport>>
where
    Transport: PromptApiTransport,
{
    match integration {
        SupportedPromptIntegrations::GenericPromptManagement => Some(prompt_initializer),
        SupportedPromptIntegrations::DotPrompt
        | SupportedPromptIntegrations::Langfuse
        | SupportedPromptIntegrations::Custom
        | SupportedPromptIntegrations::Bitbucket
        | SupportedPromptIntegrations::Gitlab
        | SupportedPromptIntegrations::ArizePhoenix => None,
    }
}
