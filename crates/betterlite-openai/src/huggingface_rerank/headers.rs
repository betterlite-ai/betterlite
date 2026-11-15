use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

const ACCEPT: &str = "accept";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "content-type";
const JSON: &str = "application/json";

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut resolved = Headers::from([
        (ACCEPT.to_owned(), JSON.to_owned()),
        (CONTENT_TYPE.to_owned(), JSON.to_owned()),
    ]);
    if let Some(key) = api_key {
        resolved.insert(
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    resolved.extend(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    resolved
}
