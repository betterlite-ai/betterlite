use secrecy::{ExposeSecret as _, SecretString};

use crate::gigachat_chat::config::GigachatBearer;
use crate::gigachat_chat::error::GigaChatError;
use crate::openai_base_config::Headers;

const ACCEPT: &str = "Accept";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";

pub struct HeaderInput<'request> {
    pub api_key: Option<&'request SecretString>,
    pub credentials: Option<&'request SecretString>,
    pub fallback_key: Option<&'request SecretString>,
    pub caller_headers: &'request Headers,
}

// LiteLLM reads api_key, then GIGACHAT_CREDENTIALS, then GIGACHAT_API_KEY, and an empty value counts as unset
fn credentials_of<'request>(input: &HeaderInput<'request>) -> Option<&'request SecretString> {
    [input.api_key, input.credentials, input.fallback_key]
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
}

// LiteLLM sets its three headers on the caller's map, so a caller header with the same name loses it
pub async fn validate_environment(
    input: &HeaderInput<'_>,
    bearer: &dyn GigachatBearer,
) -> Result<Headers, GigaChatError> {
    let token = bearer.access_token(credentials_of(input)).await?;
    let mut headers = input.caller_headers.clone();
    headers.insert(String::from(ACCEPT), String::from("application/json"));
    headers.insert(String::from(CONTENT_TYPE), String::from("application/json"));
    headers.insert(
        String::from(AUTHORIZATION),
        format!("Bearer {}", token.expose_secret()),
    );
    Ok(headers)
}
