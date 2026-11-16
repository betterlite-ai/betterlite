use secrecy::{ExposeSecret as _, SecretString};

use super::settings::{FireworksResponsesEnv, SessionSource};
use crate::fireworks_ai_common::error::MissingApiKey;
use crate::fireworks_ai_common::{
    AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, SessionParams, resolve_api_key,
    with_session_affinity,
};
use crate::openai_base_config::Headers;

const JSON_CONTENT_TYPE: &str = "application/json";

// LiteLLM's responses validate_environment: the JSON content type, then the caller's headers, then Bearer over all.
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    session: &SessionSource,
    env: &FireworksResponsesEnv,
) -> Result<Headers, MissingApiKey> {
    let key = resolve_api_key(api_key, &env.keys).ok_or(MissingApiKey)?;
    let mut authorized = Headers::new();
    authorized.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    authorized.extend(headers.clone());
    authorized.insert(
        AUTHORIZATION_HEADER.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    let params = SessionParams {
        litellm_session_id: session.litellm_session_id.clone(),
        session_id: None,
        metadata: session.litellm_metadata.clone(),
    };
    Ok(with_session_affinity(&authorized, &params))
}
