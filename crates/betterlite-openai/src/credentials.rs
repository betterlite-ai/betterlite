use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;
use url::Url;

const OPENAI_API_HOST: &str = "api.openai.com";
pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";

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
    lookup: &dyn EnvironmentLookup,
) -> String {
    let base_url = lookup.lookup("OPENAI_BASE_URL");
    let api_base_env = lookup.lookup("OPENAI_API_BASE");
    present(api_base)
        .or_else(|| present(litellm_api_base))
        .or_else(|| present(base_url.as_deref()))
        .or_else(|| present(api_base_env.as_deref()))
        .unwrap_or(DEFAULT_OPENAI_API_BASE)
        .to_owned()
}

pub fn get_openai_credentials(
    explicit: ExplicitCredentials,
    globals: LiteLlmGlobals,
    lookup: &dyn EnvironmentLookup,
) -> OpenAiCredentials {
    let api_base = resolve_openai_api_base(
        explicit.api_base.as_deref(),
        globals.api_base.as_deref(),
        lookup,
    );
    let env_organization = lookup.lookup("OPENAI_ORGANIZATION");
    let organization = present(explicit.organization.as_deref())
        .or_else(|| present(globals.organization.as_deref()))
        .or_else(|| present(env_organization.as_deref()))
        .map(str::to_owned);
    let env_api_key = lookup.lookup("OPENAI_API_KEY");
    let api_key = present_secret(explicit.api_key)
        .or_else(|| present_secret(globals.api_key))
        .or_else(|| present_secret(globals.openai_key))
        .or_else(|| present_secret(env_api_key.map(SecretString::from)));
    OpenAiCredentials {
        api_base,
        api_key,
        organization,
    }
}

pub fn is_openai_backed_api_base(api_base: &str) -> Result<bool, CredentialsError> {
    let url = Url::parse(api_base)?;
    let Some(raw_host) = url.host_str() else {
        return Ok(false);
    };
    let host = raw_host.to_ascii_lowercase();
    Ok(host == OPENAI_API_HOST || host.ends_with(&format!(".{OPENAI_API_HOST}")))
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
