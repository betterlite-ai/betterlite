use std::collections::BTreeMap;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::prompt_spec::PromptLiteLLMParams;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::arize::phoenix::caching::TemplateCache;
use crate::arize::phoenix::client::{PhoenixClient, REQUEST_DEADLINE};
use crate::arize::phoenix::integration::PhoenixPromptManager;
use crate::arize::phoenix::rendering::sandbox;

const API_KEY_VARIABLE: &str = "PHOENIX_API_KEY";

#[derive(Debug, Error)]
pub enum InitializerError {
    #[error("api_key and api_base are required for Arize Phoenix prompt integration")]
    MissingCredentials,
}

pub fn prompt_initializer(
    litellm_params: &PromptLiteLLMParams,
    environment: &BTreeMap<String, String>,
    http: ProxiedClient,
) -> Result<PhoenixPromptManager, InitializerError> {
    let configured_key = litellm_params
        .api_key
        .clone()
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            environment
                .get(API_KEY_VARIABLE)
                .filter(|key| !key.is_empty())
                .map(|key| SecretString::from(key.as_str()))
        });
    let configured_base = litellm_params
        .api_base
        .clone()
        .filter(|base| !base.is_empty());
    let (Some(api_key), Some(api_base)) = (configured_key, configured_base) else {
        return Err(InitializerError::MissingCredentials);
    };
    Ok(PhoenixPromptManager {
        client: PhoenixClient {
            http,
            api_key,
            api_base,
            deadline: REQUEST_DEADLINE,
        },
        prompt_id: litellm_params.prompt_id.clone().filter(|id| !id.is_empty()),
        templates: TemplateCache::default(),
        environment: sandbox(),
    })
}

mod tests;
