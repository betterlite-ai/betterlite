mod tests;
mod url_parts;

use std::collections::BTreeMap;
use std::str::Utf8Error;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::{ParseError, Url};

use crate::credentials::selection::{AzureCredentialRequest, AzureTokenError, get_azure_ad_token};
use crate::credentials::source::{CallableTokenProvider, CredentialSources};

#[derive(Debug, Error)]
pub enum AzureUrlError {
    #[error(
        "api_base is required for Azure AI Studio. Please set the api_base parameter. Passed `api_base=None`"
    )]
    MissingApiBase,
    #[error("api_base is not a valid URL")]
    InvalidUrl(#[source] ParseError),
    #[error("api_base contains a path that is not UTF-8")]
    InvalidPath(#[source] Utf8Error),
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

#[must_use]
pub fn is_azure_v1_api_version(api_version: Option<&str>) -> bool {
    if let Some(version) = api_version {
        return matches!(version, "v1" | "latest" | "preview");
    }
    false
}

pub fn get_base_azure_url(
    api_base: Option<&str>,
    api_version: Option<&str>,
    default_api_version: Option<&str>,
    route: &str,
) -> Result<String, AzureUrlError> {
    let Some(base) = api_base else {
        return Err(AzureUrlError::MissingApiBase);
    };
    let original = Url::parse(base).map_err(AzureUrlError::InvalidUrl)?;
    let keep_empty_path = url_parts::has_empty_path(base);
    let appended = !base.contains(route);
    let version = present(api_version).or(default_api_version);
    let mut query = url_parts::first_query_values(&original);
    if !query.iter().any(|(name, _)| name == "api-version")
        && let Some(selected) = present(version)
    {
        query.push(("api-version".to_owned(), selected.to_owned()));
    }
    let mut composed = if appended {
        Url::parse(&add_path_to_api_base(base, route)?).map_err(AzureUrlError::InvalidUrl)?
    } else {
        original
    };
    let rewritten = is_azure_v1_api_version(version) && !composed.as_str().contains("/openai/v1");
    if rewritten {
        let decoded = url_parts::path_decoded(&composed)?;
        let replacement = decoded.replace("/openai", "/openai/v1");
        composed.set_path(&replacement);
    }
    url_parts::set_query_pairs(&mut composed, &query);
    let text = composed.to_string();
    if keep_empty_path && !appended {
        return Ok(url_parts::without_empty_path(&text));
    }
    Ok(text)
}

pub fn get_azure_v1_image_url(
    api_base: &str,
    api_version: Option<&str>,
    route: &str,
) -> Result<Option<String>, AzureUrlError> {
    if !is_azure_v1_api_version(api_version) {
        return Ok(None);
    }
    let base = Url::parse(api_base).map_err(AzureUrlError::InvalidUrl)?;
    let path = url_parts::path_decoded(&base)?;
    let prefix = path
        .find("/openai")
        .map_or(path.as_str(), |index| path.get(..index).unwrap_or_default());
    let kept: Vec<(String, String)> = url_parts::query_pairs(&base)
        .into_iter()
        .filter(|(name, _)| name != "api-version")
        .collect();
    let mut resource = base;
    resource.set_path(prefix);
    url_parts::set_query_pairs(&mut resource, &kept);
    let serialized = resource.to_string();
    let resource_base = if prefix.is_empty() {
        url_parts::without_empty_path(&serialized)
    } else {
        serialized
    };
    get_base_azure_url(Some(&resource_base), api_version, None, route).map(Some)
}

pub fn resolve_env_var(
    params: &BTreeMap<String, Option<String>>,
    param_key: &str,
    env_var_key: &str,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    match params.get(param_key) {
        Some(Some(value)) => Some(value.clone()),
        Some(None) | None => environment.lookup(env_var_key),
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

const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn add_path_to_api_base(api_base: &str, ending_path: &str) -> Result<String, AzureUrlError> {
    let original = Url::parse(api_base).map_err(AzureUrlError::InvalidUrl)?;
    let path = merged_path(&original, ending_path)?;
    let mut modified = original.clone();
    modified.set_query(None);
    modified.set_path(&path);
    let query = url_parts::query_pairs(&original);
    url_parts::set_query_pairs(&mut modified, &query);
    Ok(modified.to_string())
}

fn merged_path(base: &Url, ending_path: &str) -> Result<String, AzureUrlError> {
    let base_path = url_parts::path_decoded(base)?;
    let base_segments: Vec<&str> = base_path
        .trim_end_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let end_segments: Vec<&str> = ending_path
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let overlap = (0..base_segments.len()).find(|index| {
        let tail = base_segments.get(*index..);
        let head = end_segments.get(..base_segments.len().saturating_sub(*index));
        match (tail, head) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    });
    let segments: Vec<&str> = overlap.map_or_else(
        || {
            base_segments
                .iter()
                .chain(end_segments.iter())
                .copied()
                .collect()
        },
        |index| {
            base_segments
                .iter()
                .take(index)
                .chain(end_segments.iter())
                .copied()
                .collect()
        },
    );
    Ok(format!("/{}", segments.join("/")))
}
