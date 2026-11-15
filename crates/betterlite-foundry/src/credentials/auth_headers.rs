use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret, SecretString};

use super::entra::{EntraTokenRequest, get_azure_ai_entra_token};
use super::error::CredentialsError;
use super::host::ApiKeyHeader;

pub struct AuthHeaderRequest<'data> {
    pub api_key: Option<&'data SecretString>,
    pub params: Option<&'data Object>,
    pub environment: &'data BTreeMap<String, String>,
    pub api_key_header: ApiKeyHeader,
    pub api_key_env_var: &'data str,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

pub fn get_azure_ai_auth_headers(
    request: &AuthHeaderRequest<'_>,
) -> Result<BTreeMap<String, String>, CredentialsError> {
    if let Some(api_key) = request.api_key {
        let value = match request.api_key_header {
            ApiKeyHeader::Authorization => format!("Bearer {}", api_key.expose_secret()),
            ApiKeyHeader::ApiKey
            | ApiKeyHeader::ApiKeyTitleCase
            | ApiKeyHeader::OcpApimSubscriptionKey => api_key.expose_secret().to_owned(),
        };
        return Ok(BTreeMap::from([(
            request.api_key_header.wire_name().to_owned(),
            value,
        )]));
    }
    let token = get_azure_ai_entra_token(&EntraTokenRequest {
        params: request.params,
        environment: request.environment,
        refresh_enabled: request.refresh_enabled,
        sources: request.sources,
    })?;
    let token = token.filter(|token| !token.expose_secret().is_empty());
    match token {
        Some(token) => Ok(BTreeMap::from([(
            "Authorization".to_owned(),
            format!("Bearer {}", token.expose_secret()),
        )])),
        None => Err(CredentialsError::MissingCredentials {
            env_var: request.api_key_env_var.to_owned(),
        }),
    }
}
