use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::openai_base_config::Headers;

pub const DEFAULT_BASE: &str = "https://ark.cn-beijing.volces.com";

const CONTENT_TYPE: &str = "Content-Type";
const AUTHORIZATION: &str = "Authorization";

// LiteLLM's VolcEngineError is a BaseLLMException: a status, a message and optional headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct VolcEngineError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

// an absent base takes the default while a given one is used as it is
#[must_use]
pub fn base_url(api_base: Option<&str>) -> String {
    let Some(base) = api_base else {
        return DEFAULT_BASE.to_owned();
    };
    base.to_owned()
}

// the extra headers go on top, so they win over the two defaults on the same key
#[must_use]
pub fn headers(api_key: &SecretString, extra: &Headers) -> Headers {
    let mut headers = Headers::from([
        (CONTENT_TYPE.to_owned(), String::from("application/json")),
        (
            AUTHORIZATION.to_owned(),
            format!("Bearer {}", api_key.expose_secret()),
        ),
    ]);
    headers.extend(extra.clone());
    headers
}

mod tests;
