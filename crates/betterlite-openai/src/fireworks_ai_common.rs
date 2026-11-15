pub mod error;
mod tests;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::openai_base_config::Headers;
use error::MissingApiKey;

pub const SESSION_AFFINITY_HEADER: &str = "x-session-affinity";
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone, Default)]
pub struct FireworksEnv {
    pub fireworks_api_key: Option<SecretString>,
    pub fireworks_ai_api_key: Option<SecretString>,
    pub fireworksai_api_key: Option<SecretString>,
    pub fireworks_ai_token: Option<SecretString>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SessionMetadata {
    #[serde(default)]
    pub litellm_session_id_generated: Option<bool>,
    #[serde(default)]
    pub session_id: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SessionParams {
    #[serde(default)]
    pub litellm_session_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<SessionMetadata>,
}
#[must_use]
pub fn resolve_api_key(api_key: Option<&SecretString>, env: &FireworksEnv) -> Option<SecretString> {
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        return Some(key.clone());
    }
    let candidates = [
        &env.fireworks_api_key,
        &env.fireworks_ai_api_key,
        &env.fireworksai_api_key,
        &env.fireworks_ai_token,
    ];
    candidates.into_iter().find_map(|candidate| {
        candidate
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty())
            .cloned()
    })
}
#[must_use]
pub fn get_session_id(params: &SessionParams) -> Option<String> {
    if params
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.litellm_session_id_generated)
        == Some(true)
    {
        return None;
    }
    let direct = [&params.litellm_session_id, &params.session_id];
    if let Some(session_id) = direct
        .into_iter()
        .flatten()
        .find(|session_id| !session_id.is_empty())
    {
        return Some(session_id.clone());
    }
    params
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.session_id.as_ref())
        .filter(|session_id| !session_id.is_empty())
        .cloned()
}
#[must_use]
pub fn with_session_affinity(headers: &Headers, params: &SessionParams) -> Headers {
    if headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(SESSION_AFFINITY_HEADER))
    {
        return headers.clone();
    }
    let Some(session_id) = get_session_id(params) else {
        return headers.clone();
    };
    let mut pinned = headers.clone();
    pinned.insert(SESSION_AFFINITY_HEADER.to_owned(), session_id);
    pinned
}
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env: &FireworksEnv,
    params: &SessionParams,
) -> Result<Headers, MissingApiKey> {
    let key = resolve_api_key(api_key, env).ok_or(MissingApiKey)?;
    let mut authorized = Headers::new();
    authorized.insert(
        AUTHORIZATION_HEADER.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    authorized.extend(headers.clone());
    if !authorized
        .keys()
        .any(|name| name.eq_ignore_ascii_case(CONTENT_TYPE_HEADER))
    {
        authorized.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    Ok(with_session_affinity(&authorized, params))
}
