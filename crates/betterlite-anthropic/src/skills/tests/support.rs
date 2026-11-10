use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Serialize;

use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use crate::skills::config::AnthropicSkillsConfig;

pub(super) const DEFAULT_BASE: &str = "https://api.anthropic.com";

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn settings(api_key: Option<&str>, auth_token: Option<&str>, api_base: &str) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: api_base.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(super) fn anthropic_config(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    api_base: &str,
) -> AnthropicSkillsConfig {
    AnthropicSkillsConfig {
        settings: settings(api_key, auth_token, api_base),
    }
}

pub(super) fn params(api_key: Option<&str>, api_base: Option<&str>) -> LiteLlmParams {
    LiteLlmParams {
        ssl_verify: None,
        model: None,
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(secret),
        api_version: None,
        timeout: None,
        stream_timeout: None,
        max_retries: None,
        rpm: None,
        tpm: None,
        weight: None,
    }
}

pub(super) fn header_values(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

pub(super) fn as_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn serialized<T>(value: &T) -> Json
where
    T: Serialize,
{
    as_json(&serde_json::to_string(value).unwrap())
}

pub(super) fn beta_headers(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(
            HeaderName::from_static("anthropic-beta"),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}
