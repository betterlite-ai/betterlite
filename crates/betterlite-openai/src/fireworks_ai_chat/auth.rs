use secrecy::SecretString;

use crate::chat_choices::endpoint::validate_environment as openai_headers;
use crate::fireworks_ai_chat::error::FireworksChatError;
use crate::fireworks_ai_common::error::MissingApiKey;
use crate::fireworks_ai_common::{
    FireworksEnv, SessionParams, resolve_api_key, with_session_affinity,
};
use crate::openai_base_config::Headers;

pub const DEFAULT_API_BASE: &str = "https://api.fireworks.ai/inference/v1";

// B6070's four key variables plus the base and account lookups get_models needs, empty counting as unset
#[derive(Debug, Clone, Default)]
pub struct FireworksChatEnv {
    pub keys: FireworksEnv,
    pub api_base: Option<String>,
    pub account_id: Option<String>,
}

// _get_openai_compatible_provider_info: the argument, then FIREWORKS_API_BASE, then the public base
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    env: &FireworksChatEnv,
) -> (String, Option<SecretString>) {
    let configured = env.api_base.as_deref().filter(|base| !base.is_empty());
    let base = api_base
        .or(configured)
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned();
    (base, resolve_api_key(api_key, &env.keys))
}

// validate_environment: the OpenAI chat auth rule over the resolved key, then session affinity
pub fn validate_environment(
    api_key: Option<&SecretString>,
    headers: &Headers,
    session: &SessionParams,
    env: &FireworksChatEnv,
) -> Result<Headers, FireworksChatError> {
    let key = resolve_api_key(api_key, &env.keys).ok_or(MissingApiKey)?;
    let authorized = openai_headers(Some(&key), headers);
    Ok(with_session_affinity(&authorized, session))
}
