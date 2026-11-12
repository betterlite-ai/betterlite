mod tests;
mod url_parts;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_openai::openai_base_config::forwarding::BaseConfigError;
use betterlite_openai::openai_base_config::overlapping_api_base_path;
use percent_encoding::percent_decode_str;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

use crate::credentials::selection::{AzureCredentialRequest, AzureTokenError, get_azure_ad_token};
use crate::credentials::source::{CallableTokenProvider, CredentialSources};

#[derive(Debug, Error)]
pub enum AzureUrlError {
    #[error(
        "api_base is required for Azure AI Studio. Please set the api_base parameter. Passed `api_base=None`"
    )]
    MissingApiBase,
    #[error("api_base is not a valid URL")]
    InvalidUrl(#[source] InvalidUrl),
    #[error("api_base cannot take the route")]
    Compose(#[source] BaseConfigError),
    #[error("api_base contains a path that cannot be rewritten")]
    UnrewritablePath,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointSelection {
    pub azure_endpoint: Option<String>,
    pub base_url: Option<String>,
}

#[must_use]
pub fn select_base_url_or_endpoint(selection: EndpointSelection) -> EndpointSelection {
    match selection.azure_endpoint {
        Some(endpoint) if endpoint.contains("/openai/deployments") => EndpointSelection {
            azure_endpoint: None,
            base_url: Some(endpoint),
        },
        _ => selection,
    }
}

// LiteLLM treats exactly these three version names as the v1 surface, matched case-sensitively
pub const V1_API_VERSIONS: [&str; 3] = ["preview", "latest", "v1"];

pub fn get_base_azure_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
    default_api_version: Option<&str>,
    route: &str,
) -> Result<String, AzureUrlError> {
    let Some(base) = api_base else {
        return Err(AzureUrlError::MissingApiBase);
    };
    let original = HttpxUrl::parse(base).map_err(AzureUrlError::InvalidUrl)?;
    let version = api_version
        .filter(|text| !text.is_empty())
        .or(default_api_version);
    let mut query = url_parts::first_query_values(original.query.as_deref());
    if !query.iter().any(|(name, _)| name == "api-version")
        && let Some(selected) = version.filter(|text| !text.is_empty())
    {
        query.push(("api-version".to_owned(), selected.to_owned()));
    }
    let mut composed = if base.contains(route) {
        HttpxUrl::parse(base).map_err(AzureUrlError::InvalidUrl)?
    } else {
        let merged = overlapping_api_base_path(base, route).map_err(AzureUrlError::Compose)?;
        let mut url = HttpxUrl::parse(&merged).map_err(AzureUrlError::InvalidUrl)?;
        // _add_path_to_api_base re-adds the original query through QueryParams, which re-encodes it
        url.query = url_parts::encoded_query(&url_parts::query_pairs(original.query.as_deref()));
        url
    };
    if version.is_some_and(|text| V1_API_VERSIONS.contains(&text))
        && !composed.href().contains("/openai/v1")
    {
        // httpx's URL.path reads "/" for an empty path, so every rewrite starts from a slash
        let path = if composed.path.is_empty() {
            "/"
        } else {
            composed.path.as_str()
        };
        let decoded = percent_decode_str(path).decode_utf8_lossy();
        let rewritten = decoded.replace("/openai", "/openai/v1");
        composed.path = requoted_path(&composed, &rewritten)?;
    }
    composed.query = url_parts::encoded_query(&query);
    Ok(composed.href())
}

pub fn get_azure_v1_image_url(
    api_base: &str,
    api_version: Option<&str>,
    route: &str,
) -> Result<Option<String>, AzureUrlError> {
    if !api_version.is_some_and(|text| V1_API_VERSIONS.contains(&text)) {
        return Ok(None);
    }
    let mut resource = HttpxUrl::parse(api_base).map_err(AzureUrlError::InvalidUrl)?;
    let path = if resource.path.is_empty() {
        "/"
    } else {
        resource.path.as_str()
    };
    let decoded = percent_decode_str(path).decode_utf8_lossy().into_owned();
    let prefix = decoded.find("/openai").map_or(decoded.as_str(), |index| {
        decoded.get(..index).unwrap_or_default()
    });
    let kept: Vec<(String, String)> = url_parts::query_pairs(resource.query.as_deref())
        .into_iter()
        .filter(|(name, _)| name != "api-version")
        .collect();
    resource.path = requoted_path(&resource, prefix)?;
    resource.query = url_parts::encoded_query(&kept);
    get_base_azure_url(Some(&resource.href()), api_version, None, route).map(Some)
}

// LiteLLM's auth params, resolved from the deployment or AZURE_* with the environment as fallback
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthParams {
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthEnvironment {
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
}

// `litellm_params.get(key)` wins whenever it is not None, so an explicit "" stays "" and only a missing value reads the environment
#[must_use]
pub fn resolve_auth_environment(
    params: AuthParams,
    environment: &dyn EnvironmentLookup,
) -> AuthEnvironment {
    AuthEnvironment {
        tenant_id: params
            .tenant_id
            .or_else(|| environment.lookup("AZURE_TENANT_ID")),
        client_id: params
            .client_id
            .or_else(|| environment.lookup("AZURE_CLIENT_ID")),
        client_secret: params.client_secret.or_else(|| {
            environment
                .lookup("AZURE_CLIENT_SECRET")
                .map(SecretString::from)
        }),
        azure_username: params
            .azure_username
            .or_else(|| environment.lookup("AZURE_USERNAME")),
        azure_password: params
            .azure_password
            .or_else(|| environment.lookup("AZURE_PASSWORD").map(SecretString::from)),
        azure_scope: params
            .azure_scope
            .or_else(|| environment.lookup("AZURE_SCOPE")),
    }
}

pub struct EnvironmentHeadersRequest<'data> {
    pub headers: BTreeMap<String, String>,
    pub api_key: Option<&'data SecretString>,
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

pub fn validate_azure_environment(
    request: EnvironmentHeadersRequest<'_>,
) -> Result<BTreeMap<String, String>, AzureTokenError> {
    if request.headers.contains_key("api-key") {
        return Ok(request.headers);
    }
    if let Some(key) = request
        .api_key
        .filter(|value| !value.expose_secret().is_empty())
    {
        let mut headers = request.headers;
        headers.insert("api-key".to_owned(), key.expose_secret().to_owned());
        return Ok(headers);
    }
    let token = get_azure_ad_token(&AzureCredentialRequest {
        params: request.credentials,
        refresh_enabled: request.refresh_enabled,
        provider: request.provider,
        sources: request.sources,
    })?;
    let mut headers = request.headers;
    if let Some(bearer) = token.filter(|value| !value.expose_secret().is_empty()) {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", bearer.expose_secret()),
        );
    }
    Ok(headers)
}

// httpx's copy_with(path=...) parses the decoded path again, so quoting and dot-segment removal come from HttpxUrl::parse
fn requoted_path(url: &HttpxUrl, path: &str) -> Result<String, AzureUrlError> {
    if path.contains(['?', '#']) {
        return Err(AzureUrlError::UnrewritablePath);
    }
    let mut authority = url.clone();
    authority.path = String::new();
    authority.query = None;
    authority.fragment = None;
    HttpxUrl::parse(&format!("{}{path}", authority.href()))
        .map(|composed| composed.path)
        .map_err(AzureUrlError::InvalidUrl)
}
