use std::str::from_utf8;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::fireworks_ai_chat::MODEL_PREFIX;
use crate::fireworks_ai_chat::auth::{FireworksChatEnv, provider_info};
use crate::fireworks_ai_chat::error::FireworksChatError;
use crate::openai_base_config::Headers;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const AUTHORIZATION: &str = "Authorization";
const V1_SUFFIX: &str = "/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelListRequest {
    pub url: String,
    pub headers: Headers,
}

// get_models: the base loses trailing slashes and one /v1, then the account models listing is built
pub fn get_models_request(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    env: &FireworksChatEnv,
) -> Result<ModelListRequest, FireworksChatError> {
    let (base, resolved_key) = provider_info(api_base, api_key, env);
    let Some(key) = resolved_key else {
        return Err(FireworksChatError::ModelListCredentials);
    };
    let account = env
        .account_id
        .as_deref()
        .filter(|account| !account.is_empty())
        .ok_or(FireworksChatError::AccountId)?;
    let trimmed = base.trim_end_matches('/');
    let root = trimmed.strip_suffix(V1_SUFFIX).unwrap_or(trimmed);
    let url = format!("{root}/v1/accounts/{account}/models");
    let headers = Headers::from([(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    )]);
    Ok(ModelListRequest { url, headers })
}

#[derive(Deserialize)]
struct ModelsEnvelope {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    name: String,
}

// decode_models_response: the fuzz entry point, capped before anything reads the body
pub fn decode_models_response(
    status: u16,
    bytes: &[u8],
) -> Result<Vec<String>, FireworksChatError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(FireworksChatError::ModelsTooLarge);
    }
    if status != 200 {
        let body = from_utf8(bytes).unwrap_or_default();
        return Err(FireworksChatError::ModelListStatus {
            status,
            body: body.to_owned(),
        });
    }
    let envelope: ModelsEnvelope = serde_json::from_slice(bytes)
        .map_err(|source| FireworksChatError::ModelsDecode { source })?;
    Ok(envelope
        .models
        .into_iter()
        .map(|model| format!("{MODEL_PREFIX}{}", model.name))
        .collect())
}
