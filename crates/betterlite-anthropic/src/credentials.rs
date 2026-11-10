use std::collections::BTreeSet;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

mod tests;

pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";
pub const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
pub const OAUTH_BETA_HEADER_NAME: &str = "anthropic-beta";
pub const BROWSER_ACCESS_HEADER: &str = "anthropic-dangerous-direct-browser-access";

#[derive(Debug, Clone)]
pub struct AnthropicSettings {
    pub api_key: Option<SecretString>,
    pub auth_token: Option<SecretString>,
    pub api_base: Option<String>,
    pub use_bearer_for_custom_base: bool,
    pub anthropic_version: String,
}

impl Default for AnthropicSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            auth_token: None,
            api_base: None,
            use_bearer_for_custom_base: false,
            anthropic_version: ANTHROPIC_VERSION.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthHeader {
    pub name: &'static str,
    pub value: SecretString,
}

#[derive(Debug, Clone)]
pub struct ModelsRequest {
    pub url: String,
    pub headers: IndexMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum OauthTokenClaim {
    Boolean(bool),
    Other(IgnoredAny),
}

impl Default for OauthTokenClaim {
    fn default() -> Self {
        Self::Other(IgnoredAny)
    }
}

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error(
        "ANTHROPIC_API_BASE/ANTHROPIC_BASE_URL or ANTHROPIC_API_KEY/ANTHROPIC_AUTH_TOKEN is not set. \
         Please set the environment variable, to query Anthropic's `/models` endpoint."
    )]
    MissingCredential,
    #[error("Failed to fetch models from Anthropic. Status code: {status}, Response: {response}")]
    ModelsStatus { status: u16, response: String },
    #[error("invalid Anthropic models response: {source}")]
    ModelsJson {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn is_anthropic_oauth_key(value: Option<&str>) -> bool {
    let Some(text) = value else {
        return false;
    };
    text.strip_prefix("Bearer ")
        .unwrap_or(text)
        .starts_with(OAUTH_TOKEN_PREFIX)
}

#[must_use]
pub fn resolve_oauth_token_claim(
    client_sent_oauth_token: OauthTokenClaim,
    custom_llm_provider: Option<&str>,
) -> Option<bool> {
    match client_sent_oauth_token {
        OauthTokenClaim::Other(_) => None,
        OauthTokenClaim::Boolean(sent) => Some(sent && custom_llm_provider == Some("anthropic")),
    }
}

#[must_use]
pub fn merge_beta_headers(existing: Option<&str>, new_beta: &str) -> String {
    let Some(existing_betas) = existing.filter(|value| !value.is_empty()) else {
        return new_beta.to_owned();
    };
    let mut betas: BTreeSet<&str> = existing_betas
        .split(',')
        .map(str::trim)
        .filter(|beta| !beta.is_empty())
        .collect();
    betas.insert(new_beta);
    betas.into_iter().collect::<Vec<_>>().join(",")
}

pub fn handle_anthropic_oauth(
    headers: &mut IndexMap<String, String>,
    api_key: Option<SecretString>,
) -> Option<SecretString> {
    let auth_header = headers
        .iter()
        .find(|(name, _)| name.to_lowercase() == "authorization")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    if let Some(token) = auth_header
        .strip_prefix("Bearer ")
        .filter(|token| token.starts_with(OAUTH_TOKEN_PREFIX))
    {
        let resolved = SecretString::from(token.to_owned());
        headers.retain(|name, _| {
            !matches!(name.to_lowercase().as_str(), "authorization" | "x-api-key")
        });
        headers.insert("authorization".to_owned(), auth_header);
        apply_oauth_markers(headers);
        return Some(resolved);
    }
    let resolved_key = api_key?;
    if resolved_key.expose_secret().starts_with(OAUTH_TOKEN_PREFIX) {
        headers.retain(|name, _| name.to_lowercase() != "x-api-key");
        headers.insert(
            "authorization".to_owned(),
            format!("Bearer {}", resolved_key.expose_secret()),
        );
        apply_oauth_markers(headers);
    }
    Some(resolved_key)
}

fn apply_oauth_markers(headers: &mut IndexMap<String, String>) {
    let beta = merge_beta_headers(
        headers.get(OAUTH_BETA_HEADER_NAME).map(String::as_str),
        OAUTH_BETA_HEADER,
    );
    headers.insert(OAUTH_BETA_HEADER_NAME.to_owned(), beta);
    headers.insert(BROWSER_ACCESS_HEADER.to_owned(), "true".to_owned());
}

#[must_use]
pub fn make_api_key_auth_header(
    api_key: &SecretString,
    api_base: Option<&str>,
    use_bearer_for_custom_base: bool,
) -> AuthHeader {
    let secret = api_key.expose_secret();
    let custom_base =
        api_base.is_some_and(|base| !base.is_empty() && !base.contains("api.anthropic.com"));
    if use_bearer_for_custom_base && custom_base && !secret.starts_with("sk-ant-") {
        let value = if secret.starts_with("Bearer ") {
            secret.to_owned()
        } else {
            format!("Bearer {secret}")
        };
        return AuthHeader {
            name: "authorization",
            value: SecretString::from(value),
        };
    }
    AuthHeader {
        name: "x-api-key",
        value: api_key.clone(),
    }
}

#[must_use]
pub fn get_api_base(api_base: Option<&str>, settings: &AnthropicSettings) -> String {
    api_base
        .map(str::to_owned)
        .or_else(|| settings.api_base.clone())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
}

#[must_use]
pub fn get_auth_header(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    use_bearer_for_custom_base: bool,
    settings: &AnthropicSettings,
) -> Option<AuthHeader> {
    if let Some(resolved_key) = api_key.or(settings.api_key.as_ref()) {
        if is_anthropic_oauth_key(Some(resolved_key.expose_secret())) {
            return Some(AuthHeader {
                name: "authorization",
                value: SecretString::from(format!("Bearer {}", resolved_key.expose_secret())),
            });
        }
        return Some(make_api_key_auth_header(
            resolved_key,
            api_base,
            use_bearer_for_custom_base,
        ));
    }
    let auth_token = settings.auth_token.as_ref()?;
    Some(AuthHeader {
        name: "authorization",
        value: SecretString::from(format!("Bearer {}", auth_token.expose_secret())),
    })
}

#[must_use]
pub fn get_base_model(model: Option<&str>) -> Option<String> {
    model
        .filter(|name| !name.is_empty())
        .map(|name| name.replace("anthropic/", ""))
}

pub fn models_request(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    settings: &AnthropicSettings,
) -> Result<ModelsRequest, CredentialsError> {
    let base = get_api_base(api_base, settings);
    let auth_header = get_auth_header(api_key, Some(&base), false, settings)
        .ok_or(CredentialsError::MissingCredential)?;
    let mut headers = IndexMap::new();
    headers.insert(
        "anthropic-version".to_owned(),
        settings.anthropic_version.clone(),
    );
    headers.insert(
        auth_header.name.to_owned(),
        auth_header.value.expose_secret().to_owned(),
    );
    Ok(ModelsRequest {
        url: format!("{base}/v1/models"),
        headers,
    })
}

pub fn decode_anthropic_models(status: u16, bytes: &[u8]) -> Result<Vec<String>, CredentialsError> {
    if !(200..300).contains(&status) {
        let length = bytes.len();
        let response = str::from_utf8(bytes).map_or_else(
            |_| format!("{length} bytes that are not UTF-8"),
            str::to_owned,
        );
        return Err(CredentialsError::ModelsStatus { status, response });
    }
    let list: ModelList =
        serde_json::from_slice(bytes).map_err(|source| CredentialsError::ModelsJson { source })?;
    Ok(list
        .data
        .into_iter()
        .map(|model| format!("anthropic/{}", model.id))
        .collect())
}

#[derive(Debug, Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[must_use]
pub fn anthropic_error(
    status_code: u16,
    message: String,
    headers: Option<Vec<(String, String)>>,
) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message,
        headers,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
