use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_urls::{
    AuthParams, AzureUrlError, EnvironmentHeadersRequest, get_azure_v1_image_url,
    get_base_azure_url, resolve_auth_environment, validate_azure_environment,
};
use crate::credentials::selection::AzureTokenError;
use crate::credentials::source::{CallableTokenProvider, CredentialSources};

const DEPLOYMENTS: &str = "/openai/deployments/";
const EDITS_ROUTE: &str = "/openai/images/edits";

// Azure routes an edit by the deployment in the URL, so the form never repeats the model there
#[must_use]
pub fn finalize_image_edit_request_data(
    data: &IndexMap<String, Json>,
    request_url: &str,
) -> IndexMap<String, Json> {
    let mut data = data.clone();
    if request_url.contains("images/edits") && request_url.contains(DEPLOYMENTS) {
        data.shift_remove("model");
    }
    data
}

// the litellm_params keys the edit auth reads; the positional api_key fills its key only when unset
#[derive(Debug, Default, Clone)]
pub struct EditParams {
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub auth: AuthParams,
}

pub struct EditAuth<'data> {
    pub headers: BTreeMap<String, String>,
    pub api_key: Option<&'data SecretString>,
    pub params: &'data EditParams,
    pub credentials: &'data AzureCredentials,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
}

pub fn validate_environment(
    request: &EditAuth<'_>,
) -> Result<BTreeMap<String, String>, AzureTokenError> {
    let api_key = request
        .params
        .api_key
        .clone()
        .or_else(|| request.api_key.cloned())
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            request
                .credentials
                .api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
                .cloned()
        });
    let auth = resolve_auth_environment(request.params.auth.clone(), request.environment);
    let credentials = DeploymentSettings {
        azure_ad_token: azure_ad_token(request),
        tenant_id: auth.tenant_id,
        client_id: auth.client_id,
        client_secret: auth.client_secret,
        azure_scope: auth.azure_scope.unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
        azure_username: auth.azure_username,
        azure_password: auth.azure_password,
    };
    validate_azure_environment(EnvironmentHeadersRequest {
        headers: request.headers.clone(),
        api_key: api_key.as_ref(),
        credentials: &credentials,
        refresh_enabled: request.refresh_enabled,
        provider: request.provider.clone(),
        sources: request.sources,
    })
}

// LiteLLM reads litellm_params.azure_ad_token or AZURE_AD_TOKEN, so an empty value falls through
fn azure_ad_token(request: &EditAuth<'_>) -> Option<SecretString> {
    let stored = request.params.azure_ad_token.clone();
    let environment = request
        .environment
        .lookup("AZURE_AD_TOKEN")
        .filter(|token| !token.is_empty())
        .map(SecretString::from);
    stored
        .filter(|token| !token.expose_secret().is_empty())
        .or(environment)
}

pub struct EditUrl<'data> {
    pub model: &'data str,
    pub api_base: Option<&'data str>,
    pub api_version: Option<&'data str>,
    pub credentials: &'data AzureCredentials,
    pub default_api_version: &'data str,
}

pub fn get_complete_url(request: &EditUrl<'_>) -> Result<String, AzureUrlError> {
    let api_base = request
        .api_base
        .filter(|text| !text.is_empty())
        .or(request.credentials.api_base.as_deref())
        .ok_or(AzureUrlError::MissingApiBase)?;
    let api_version = request
        .api_version
        .filter(|text| !text.is_empty())
        .or(request
            .credentials
            .api_version
            .as_deref()
            .filter(|text| !text.is_empty()))
        .unwrap_or(request.default_api_version);
    if let Some(url) = get_azure_v1_image_url(api_base, Some(api_version), EDITS_ROUTE)? {
        return Ok(url);
    }
    let route = if api_base.contains(DEPLOYMENTS) {
        String::new()
    } else {
        format!("{DEPLOYMENTS}{}/images/edits", request.model)
    };
    get_base_azure_url(Some(api_base), Some(api_version), None, &route)
}
