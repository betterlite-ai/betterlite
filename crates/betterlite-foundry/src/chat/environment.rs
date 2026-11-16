use std::fmt::{self, Formatter};
use std::sync::Arc;

use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use betterlite_azure::credentials::source::{CallableTokenProvider, CredentialSources};
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::environment_error::EnvironmentError;
use crate::credentials::host::{ApiKeyHeader, api_key_header_for_base};

const API_KEY_HEADER: &str = "api-key";
const AUTHORIZATION_HEADER: &str = "Authorization";
const BEARER_PREFIX: &str = "Bearer ";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Clone)]
pub struct TokenSource<'data> {
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

// configured_key is litellm.api_key, litellm.azure_key and the Azure key variables, resolved by the caller
#[derive(Clone)]
pub struct EnvironmentRequest<'data> {
    pub headers: Headers,
    pub argument_key: Option<&'data SecretString>,
    pub params_key: Option<&'data SecretString>,
    pub configured_key: Option<&'data SecretString>,
    pub token: TokenSource<'data>,
    pub api_base: Option<&'data str>,
}

// Debug prints which credentials the request carries, never their values.
impl fmt::Debug for EnvironmentRequest<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentRequest")
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("has_argument_key", &self.argument_key.is_some())
            .field("has_params_key", &self.params_key.is_some())
            .field("has_configured_key", &self.configured_key.is_some())
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

pub fn validate_environment(request: EnvironmentRequest<'_>) -> Result<Headers, EnvironmentError> {
    let mut headers = request.headers;
    let argument = request
        .argument_key
        .filter(|key| !key.expose_secret().is_empty());
    match argument {
        Some(key) => {
            let takes_api_key = request
                .api_base
                .is_some_and(|base| api_key_header_for_base(Some(base)) == ApiKeyHeader::ApiKey);
            if takes_api_key {
                headers.insert(String::from(API_KEY_HEADER), key.expose_secret().to_owned());
            } else {
                headers.insert(
                    String::from(AUTHORIZATION_HEADER),
                    format!("{BEARER_PREFIX}{}", key.expose_secret()),
                );
            }
        }
        None => {
            let resolved = request
                .params_key
                .filter(|key| !key.expose_secret().is_empty())
                .or(request
                    .configured_key
                    .filter(|key| !key.expose_secret().is_empty()));
            headers = validate_azure_environment(EnvironmentHeadersRequest {
                headers,
                api_key: resolved,
                credentials: request.token.credentials,
                refresh_enabled: request.token.refresh_enabled,
                provider: request.token.provider,
                sources: request.token.sources,
            })?;
        }
    }
    // LiteLLM assigns the header, so an earlier Content-Type of another case is overwritten
    headers.insert(
        String::from(CONTENT_TYPE_HEADER),
        String::from(JSON_CONTENT_TYPE),
    );
    Ok(headers)
}
