use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::AZURE_AI_API_KEY_VAR;
use crate::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use crate::credentials::error::CredentialsError;
use crate::credentials::host::api_key_header_for_base;

pub struct ValidateEnvironmentRequest<'data> {
    pub headers: Headers,
    pub api_key: Option<&'data SecretString>,
    pub params: Option<&'data Object>,
    pub api_base: Option<&'data str>,
    pub environment: &'data BTreeMap<String, String>,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

// {**headers, **get_azure_ai_auth_headers(...)}: an auth header replaces a caller header of the same name
pub fn validate_environment(
    request: ValidateEnvironmentRequest<'_>,
) -> Result<Headers, CredentialsError> {
    // LiteLLM's auth helper reads the key for truth, so an empty one falls to Entra
    let api_key = request
        .api_key
        .filter(|key| !key.expose_secret().is_empty());
    let auth = get_azure_ai_auth_headers(&AuthHeaderRequest {
        api_key,
        params: request.params,
        environment: request.environment,
        api_key_header: api_key_header_for_base(request.api_base),
        api_key_env_var: AZURE_AI_API_KEY_VAR,
        refresh_enabled: request.refresh_enabled,
        sources: request.sources,
    })?;
    let mut headers = request.headers;
    headers.extend(auth);
    Ok(headers)
}
