pub mod error;
mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use error::CloudflareChatError;

const ACCOUNT_BASE: &str = "https://api.cloudflare.com/client/v4/accounts";
const LEGACY_PATH: &str = "/ai/run";
const OPENAI_PATH: &str = "/ai/v1";
const ENDPOINT: &str = "chat/completions";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const LOWER_CONTENT_TYPE: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";

pub fn complete_url(
    api_base: Option<&str>,
    account_id: Option<&str>,
) -> Result<String, CloudflareChatError> {
    let base = match api_base {
        Some(base) if !base.is_empty() => explicit_base(base),
        _ => account_base(account_id)?,
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.contains(ENDPOINT) {
        return Ok(trimmed.to_owned());
    }
    Ok(format!("{trimmed}/{ENDPOINT}"))
}

fn explicit_base(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    let Some(root) = trimmed.strip_suffix(LEGACY_PATH) else {
        return api_base.to_owned();
    };
    tracing::warn!(
        "Cloudflare api_base ending in '/ai/run' is the legacy Workers AI path and no longer serves OpenAI-compatible requests; rewriting to the '/ai/v1' endpoint"
    );
    format!("{root}{OPENAI_PATH}")
}

fn account_base(account_id: Option<&str>) -> Result<String, CloudflareChatError> {
    let id = account_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or(CloudflareChatError::MissingAccountId)?;
    Ok(format!("{ACCOUNT_BASE}/{id}{OPENAI_PATH}"))
}

pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
) -> Result<Headers, CloudflareChatError> {
    let key = api_key.ok_or(CloudflareChatError::MissingApiKey)?;
    headers.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    if !headers.contains_key(LOWER_CONTENT_TYPE) && !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    Ok(headers)
}
