use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::AZURE_AI_API_KEY_VAR;
use crate::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use crate::credentials::host::ApiKeyHeader;

use super::error::FoundryRerankError;

pub struct ValidateEnvironmentRequest<'data> {
    pub headers: Headers,
    pub api_key: Option<&'data SecretString>,
    pub params: Option<&'data Object>,
    pub environment: &'data BTreeMap<String, String>,
    pub global_api_key: Option<&'data SecretString>,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

// LiteLLM reads the variable and the global key only when the caller gave no key at all; an explicit empty key stays empty and falls to Entra
pub fn validate_environment(
    request: &ValidateEnvironmentRequest<'_>,
) -> Result<Headers, FoundryRerankError> {
    let api_key = match request.api_key {
        Some(key) if !key.expose_secret().is_empty() => Some(key.clone()),
        Some(_) => None,
        None => request
            .environment
            .get(AZURE_AI_API_KEY_VAR)
            .filter(|text| !text.is_empty())
            .map(|text| SecretString::from(text.clone()))
            .or_else(|| {
                request
                    .global_api_key
                    .filter(|key| !key.expose_secret().is_empty())
                    .cloned()
            }),
    };
    let mut headers = get_azure_ai_auth_headers(&AuthHeaderRequest {
        api_key: api_key.as_ref(),
        params: request.params,
        environment: request.environment,
        api_key_header: ApiKeyHeader::Authorization,
        api_key_env_var: AZURE_AI_API_KEY_VAR,
        refresh_enabled: request.refresh_enabled,
        sources: request.sources,
    })?;
    headers.insert("accept".to_owned(), "application/json".to_owned());
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    headers.extend(request.headers.clone());
    Ok(headers)
}
