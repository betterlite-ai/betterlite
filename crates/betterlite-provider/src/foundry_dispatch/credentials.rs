use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::credentials::source::{CallableTokenProvider, CredentialSources};
use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use betterlite_foundry::credentials::entra::{EntraTokenRequest, get_azure_ai_entra_token};
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

// litellm.api_base, litellm.api_key, litellm.headers and the AZURE_AI_* variables, read by the caller
pub struct Globals<'data> {
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub environment_api_base: Option<&'data str>,
    pub environment_api_key: Option<&'data SecretString>,
    pub headers: Option<&'data Headers>,
    pub environment: &'data BTreeMap<String, String>,
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
    pub token_provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
}

impl Globals<'_> {
    pub fn accessors(&self) -> ModelInfoAccessors<'_> {
        ModelInfoAccessors {
            global_api_base: self.api_base.filter(|text| !text.is_empty()),
            global_api_key: self.api_key.filter(|key| !key.expose_secret().is_empty()),
            environment_api_base: self.environment_api_base.filter(|text| !text.is_empty()),
            environment_api_key: self
                .environment_api_key
                .filter(|key| !key.expose_secret().is_empty()),
        }
    }
}

#[must_use]
pub fn resolve_key(explicit: Option<&SecretString>, globals: &Globals<'_>) -> Option<SecretString> {
    globals
        .accessors()
        .api_key(explicit.filter(|key| !key.expose_secret().is_empty()))
}

// LiteLLM's embedding call falls back to an Entra token for the API-key slot once the three strings are unset
pub fn resolve_embedding_key(
    explicit: Option<&SecretString>,
    params: &Object,
    globals: &Globals<'_>,
) -> Result<Option<SecretString>, CredentialsError> {
    if let Some(key) = resolve_key(explicit, globals) {
        return Ok(Some(key));
    }
    let token = get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(params),
        environment: globals.environment,
        refresh_enabled: globals.refresh_enabled,
        sources: globals.sources,
    })?;
    Ok(token.filter(|key| !key.expose_secret().is_empty()))
}

// LiteLLM reads `headers or litellm.headers`, so an empty map falls back like an absent one
#[must_use]
pub fn headers_or_global<'data>(
    supplied: Option<&'data Headers>,
    globals: &'data Globals<'_>,
) -> Option<&'data Headers> {
    supplied
        .filter(|headers| !headers.is_empty())
        .or(globals.headers)
}

#[must_use]
pub fn secret_headers(headers: Option<&Headers>) -> BTreeMap<String, SecretString> {
    headers.map_or_else(BTreeMap::new, |given| {
        given
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect()
    })
}

// The azure_credentials identity of a deployment that configures no Azure AD values at all
#[must_use]
pub fn no_credentials() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from(DEFAULT_SCOPE),
        azure_username: None,
        azure_password: None,
    }
}
