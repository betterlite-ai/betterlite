use std::collections::BTreeMap;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::prompt_spec::PromptLiteLLMParams;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::arize::phoenix::caching::TemplateCache;
use crate::arize::phoenix::client::{ClientError, PhoenixClient};
use crate::arize::phoenix::integration::PhoenixPromptManager;
use crate::arize::phoenix::rendering::sandbox;

const API_KEY_VARIABLE: &str = "PHOENIX_API_KEY";

#[derive(Debug, Error)]
pub enum InitializerError {
    #[error("api_key and api_base are required for Arize Phoenix prompt integration")]
    MissingCredentials,
    #[error(transparent)]
    Client(#[from] ClientError),
}

pub fn prompt_initializer(
    litellm_params: &PromptLiteLLMParams,
    environment: &BTreeMap<String, String>,
    http: ProxiedClient,
) -> Result<PhoenixPromptManager, InitializerError> {
    let api_key = litellm_params
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            environment
                .get(API_KEY_VARIABLE)
                .filter(|key| !key.is_empty())
                .map(|key| SecretString::from(key.as_str()))
        });
    let api_base = litellm_params
        .api_base
        .clone()
        .filter(|base| !base.is_empty());
    if api_key.is_none() || api_base.is_none() {
        return Err(InitializerError::MissingCredentials);
    }
    Ok(PhoenixPromptManager {
        client: PhoenixClient::new(http, api_key, api_base)?,
        prompt_id: litellm_params.prompt_id.clone(),
        templates: TemplateCache::default(),
        environment: sandbox(),
    })
}

mod tests;
