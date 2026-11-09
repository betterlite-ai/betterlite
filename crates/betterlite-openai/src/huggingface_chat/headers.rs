use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;

const CONTENT_TYPE: &str = "content-type";
const AUTHORIZATION: &str = "Authorization";

#[must_use]
pub fn validate_environment(headers: &Headers, api_key: Option<&SecretString>) -> Headers {
    let mut defaults = Headers::from([(CONTENT_TYPE.to_owned(), "application/json".to_owned())]);
    if let Some(key) = api_key {
        defaults.insert(
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    let mut merged = headers.clone();
    merged.extend(defaults);
    merged
}
