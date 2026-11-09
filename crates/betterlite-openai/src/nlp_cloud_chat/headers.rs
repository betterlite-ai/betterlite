use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

const ACCEPT: &str = "accept";
const CONTENT_TYPE: &str = "content-type";
const AUTHORIZATION: &str = "Authorization";

// LiteLLM replaces the caller's header map instead of extending it
#[must_use]
pub fn validate_environment(_caller_headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut headers = Headers::from([
        (ACCEPT.to_owned(), "application/json".to_owned()),
        (CONTENT_TYPE.to_owned(), "application/json".to_owned()),
    ]);
    if let Some(key) = api_key {
        headers.insert(
            AUTHORIZATION.to_owned(),
            format!("Token {}", key.expose_secret()),
        );
    }
    headers
}
