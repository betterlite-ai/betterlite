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

pub async fn validate_environment(
    input: &HeaderInput<'_>,
    bearer: &dyn GigachatBearer,
) -> Result<Headers, GigaChatError> {
    let credentials = [input.api_key, input.credentials, input.fallback_key]
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty());
    let token = bearer.access_token(credentials).await?;
    let mut headers = input.caller_headers.clone();
    headers.insert(String::from(ACCEPT), String::from("application/json"));
    headers.insert(String::from(CONTENT_TYPE), String::from("application/json"));
    headers.insert(
        String::from(AUTHORIZATION),
        format!("Bearer {}", token.expose_secret()),
    );
    Ok(headers)
}
