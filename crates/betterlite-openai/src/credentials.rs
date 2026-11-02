use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;
use url::Url;

const OPENAI_API_HOST: &str = "api.openai.com";

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("api_base is not a valid URL")]
    InvalidApiBase(#[from] url::ParseError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataResidency {
    Eu,
    Us,
}

#[derive(Debug, Default)]
pub struct ExplicitCredentials {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug, Default)]
pub struct LiteLlmGlobals {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug)]
pub struct OpenAiCredentials {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

pub fn resolve_openai_api_base(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    settings: &OpenAiCredentialsSettings,
) -> String {
    present(api_base)
        .or_else(|| present(litellm_api_base))
        .map_or_else(|| settings.api_base.clone(), str::to_owned)
}

pub fn get_openai_credentials(
    explicit: ExplicitCredentials,
    globals: LiteLlmGlobals,
    settings: &OpenAiCredentialsSettings,
    env_api_key: Option<SecretString>,
) -> OpenAiCredentials {
    let api_base = resolve_openai_api_base(
        explicit.api_base.as_deref(),
        globals.api_base.as_deref(),
        settings,
    );
    let organization = present(explicit.organization.as_deref())
        .or_else(|| present(globals.organization.as_deref()))
        .or_else(|| present(settings.organization.as_deref()))
        .map(str::to_owned);
    let api_key = present_secret(explicit.api_key)
        .or_else(|| present_secret(globals.api_key))
        .or_else(|| present_secret(globals.openai_key))
        .or_else(|| present_secret(env_api_key));
    OpenAiCredentials {
        api_base,
        api_key,
        organization,
    }
}

pub fn is_openai_backed_api_base(api_base: &str) -> Result<bool, CredentialsError> {
    let host = extract_host(api_base)?;
    Ok(host.is_some_and(|name| {
        name == OPENAI_API_HOST || name.ends_with(&format!(".{OPENAI_API_HOST}"))
    }))
}

fn extract_host(api_base: &str) -> Result<Option<String>, CredentialsError> {
    match Url::parse(api_base) {
        Ok(url) => Ok(url.host_str().map(str::to_ascii_lowercase)),
        Err(url::ParseError::EmptyHost | url::ParseError::RelativeUrlWithoutBase) => Ok(None),
        Err(url::ParseError::InvalidPort) => Ok(host_ignoring_port(api_base)),
        Err(error) => Err(CredentialsError::InvalidApiBase(error)),
    }
}

// urlsplit().hostname never validates the port, unlike url::Url::parse; strip it and retry.
fn host_ignoring_port(api_base: &str) -> Option<String> {
    let (scheme, rest) = api_base.split_once("://")?;
    let (authority, suffix) = rest
        .find(['/', '?', '#'])
        .map_or((rest, ""), |boundary| rest.split_at(boundary));
    let (host, port) = authority.rsplit_once(':')?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let stripped = format!("{scheme}://{host}{suffix}");
    Url::parse(&stripped)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

#[must_use]
pub fn infer_openai_data_residency(
    custom_llm_provider: Option<&str>,
    api_base: Option<&str>,
) -> Option<DataResidency> {
    if custom_llm_provider != Some("openai") {
        return None;
    }
    let host = present(api_base)
        .and_then(|base| Url::parse(base).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))?;
    match host.as_str() {
        "eu.api.openai.com" => Some(DataResidency::Eu),
        "us.api.openai.com" => Some(DataResidency::Us),
        _ => None,
    }
}

const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn present_secret(value: Option<SecretString>) -> Option<SecretString> {
    match value {
        Some(secret) if !secret.expose_secret().is_empty() => Some(secret),
        _ => None,
    }
}

mod tests;
