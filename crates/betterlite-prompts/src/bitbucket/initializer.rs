use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::{
    PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations,
};
use thiserror::Error;

use crate::bitbucket_client::config::BitBucketConfig;

use super::manager::BitBucketPromptManager;

#[derive(Debug, Error)]
pub enum BitBucketInitializerError {
    #[error("bitbucket_config is required for BitBucket prompt integration")]
    MissingConfig,
    #[error("litellm_params.bitbucket_config is not a valid BitBucket configuration: {source}")]
    InvalidConfig {
        #[source]
        source: serde_json::Error,
    },
}

pub type InitializerFn = fn(
    &PromptLiteLLMParams,
    &PromptSpec,
    ProxiedClient,
) -> Result<BitBucketPromptManager, BitBucketInitializerError>;

pub fn prompt_initializer(
    litellm_params: &PromptLiteLLMParams,
    _prompt_spec: &PromptSpec,
    transport: ProxiedClient,
) -> Result<BitBucketPromptManager, BitBucketInitializerError> {
    // LiteLLM reads both fields off the model, where pydantic keeps bitbucket_config as an extra
    let config = bitbucket_config(litellm_params)?;
    Ok(BitBucketPromptManager::new(
        config,
        litellm_params.prompt_id.clone(),
        transport,
    ))
}

#[must_use]
pub fn prompt_initializer_registry(
    integration: &SupportedPromptIntegrations,
) -> Option<InitializerFn> {
    match integration {
        SupportedPromptIntegrations::Bitbucket => Some(prompt_initializer),
        SupportedPromptIntegrations::DotPrompt
        | SupportedPromptIntegrations::Langfuse
        | SupportedPromptIntegrations::Custom
        | SupportedPromptIntegrations::Gitlab
        | SupportedPromptIntegrations::GenericPromptManagement
        | SupportedPromptIntegrations::ArizePhoenix => None,
    }
}

fn bitbucket_config(
    litellm_params: &PromptLiteLLMParams,
) -> Result<BitBucketConfig, BitBucketInitializerError> {
    let Some(value) = litellm_params.extra.get("bitbucket_config") else {
        return Err(BitBucketInitializerError::MissingConfig);
    };
    if matches!(value, ContractJson::Null) {
        return Err(BitBucketInitializerError::MissingConfig);
    }
    // the mapping arrives as JSON, and the typed config is where LiteLLM's client reads it
    let text = serde_json::to_string(value)
        .map_err(|source| BitBucketInitializerError::InvalidConfig { source })?;
    serde_json::from_str(&text)
        .map_err(|source| BitBucketInitializerError::InvalidConfig { source })
}
