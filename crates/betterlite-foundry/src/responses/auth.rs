use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::AZURE_AI_API_KEY_VAR;
use crate::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use crate::credentials::error::CredentialsError;
use crate::credentials::host::api_key_header_for_base;
use crate::credentials::model_info::ModelInfoAccessors;

pub struct EnvironmentRequest<'data> {
    pub headers: &'data BTreeMap<String, String>,
    pub accessors: ModelInfoAccessors<'data>,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub params: Option<&'data Object>,
    pub environment: &'data BTreeMap<String, String>,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

pub fn validate_environment(
    request: &EnvironmentRequest<'_>,
) -> Result<BTreeMap<String, String>, CredentialsError> {
    let resolved_base = request.accessors.api_base(request.api_base);
    // LiteLLM's get_azure_ai_auth_headers reads the key for truth, so an empty one falls to Entra
    let api_key = request
        .accessors
        .api_key(request.api_key)
        .filter(|key| !key.expose_secret().is_empty());
    let auth = get_azure_ai_auth_headers(&AuthHeaderRequest {
        api_key: api_key.as_ref(),
        params: request.params,
        environment: request.environment,
        api_key_header: api_key_header_for_base(resolved_base.as_deref()),
        api_key_env_var: AZURE_AI_API_KEY_VAR,
        refresh_enabled: request.refresh_enabled,
        sources: request.sources,
    })?;
    let mut headers = request.headers.clone();
    headers.extend(auth);
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    Ok(headers)
}
