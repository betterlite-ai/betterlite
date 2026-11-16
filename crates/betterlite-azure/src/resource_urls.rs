pub mod error;

mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_http::user_url::split::{Parsed, urlparse};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::map_params;
use betterlite_openai::videos::create::VideoCreate;
use secrecy::{ExposeSecret as _, SecretString};
use url::form_urlencoded;

use crate::azure_urls::{
    EnvironmentHeadersRequest, get_base_azure_url, validate_azure_environment,
};
use crate::credentials::source::{CallableTokenProvider, CredentialSources};

use error::ResourceUrlError;

// Endpoint paths a deployment's api_base may carry; the containers URL starts from the resource root
const CONTAINER_ENDPOINT_PATHS: [&str; 1] = ["/openai/responses"];

// Azure video leaves out `characters`, which the OpenAI list advertises
pub const VIDEO_SUPPORTED_PARAMS: [&str; 7] = [
    "model",
    "prompt",
    "input_reference",
    "seconds",
    "size",
    "user",
    "extra_headers",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureResource {
    Container,
    Video,
    VectorStore,
}

#[derive(Debug, Clone, Copy)]
pub struct UrlRequest<'data> {
    pub api_base: Option<&'data str>,
    // litellm.api_base, else AZURE_API_BASE: LiteLLM reads them after the container has changed api_base
    pub fallback_api_base: Option<&'data str>,
    pub api_version: Option<&'data str>,
}

pub struct ResourceAuth<'data> {
    pub headers: BTreeMap<String, String>,
    // litellm.api_key, litellm.azure_key, AZURE_OPENAI_API_KEY, else AZURE_API_KEY
    pub fallback_api_key: Option<&'data SecretString>,
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

fn container_api_version(api_base: &str) -> Result<Option<String>, ResourceUrlError> {
    let query = urlparse(api_base, "")
        .map_err(ResourceUrlError::Parse)?
        .query;
    Ok(form_urlencoded::parse(query.as_bytes())
        .find(|(name, value)| name == "api-version" && !value.is_empty())
        .map(|(_, value)| value.into_owned()))
}

fn container_resource_root(api_base: &str) -> Result<String, ResourceUrlError> {
    let parsed = urlparse(api_base, "").map_err(ResourceUrlError::Parse)?;
    let trimmed = parsed.path.trim_end_matches('/');
    for ending in CONTAINER_ENDPOINT_PATHS {
        if let Some(root) = trimmed.strip_suffix(ending) {
            return Ok(Parsed {
                scheme: parsed.scheme.clone(),
                netloc: parsed.netloc.clone(),
                path: root.to_owned(),
                params: String::new(),
                query: String::new(),
                fragment: String::new(),
            }
            .unparse());
        }
    }
    Ok(api_base.to_owned())
}

pub fn complete_url(
    resource: AzureResource,
    request: &UrlRequest<'_>,
) -> Result<String, ResourceUrlError> {
    let (route, default_version) = match resource {
        AzureResource::Container => ("/openai/containers", Some("v1")),
        AzureResource::Video => ("/openai/v1/videos", Some("")),
        AzureResource::VectorStore => ("/openai/vector_stores", None),
    };
    let mut api_base = request.api_base.map(str::to_owned);
    let mut api_version = request.api_version.map(str::to_owned);
    if resource == AzureResource::Container
        && let Some(base) = request.api_base.filter(|text| !text.is_empty())
    {
        api_version = container_api_version(base)?.or(api_version);
        api_base = Some(container_resource_root(base)?);
    }
    let base = api_base
        .as_deref()
        .filter(|text| !text.is_empty())
        .or(request.fallback_api_base);
    get_base_azure_url(base, api_version.as_deref(), default_version, route)
        .map_err(ResourceUrlError::Url)
}

// Containers pass their positional key as the params key, and vector stores pass the params they were given
pub fn validate_environment<'data>(
    auth: ResourceAuth<'data>,
    params_api_key: Option<&'data SecretString>,
) -> Result<BTreeMap<String, String>, ResourceUrlError> {
    let api_key = params_api_key
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .or(auth.fallback_api_key);
    validate_azure_environment(EnvironmentHeadersRequest {
        headers: auth.headers,
        api_key,
        credentials: auth.credentials,
        refresh_enabled: auth.refresh_enabled,
        provider: auth.provider,
        sources: auth.sources,
    })
    .map_err(ResourceUrlError::Token)
}

// A truthy positional key fills a falsy params key in the caller's params, which stay filled afterwards
pub fn video_validate_environment(
    auth: ResourceAuth<'_>,
    api_key: Option<&SecretString>,
    params_api_key: &mut Option<SecretString>,
) -> Result<BTreeMap<String, String>, ResourceUrlError> {
    if let Some(key) = api_key.filter(|candidate| !candidate.expose_secret().is_empty())
        && params_api_key
            .as_ref()
            .is_none_or(|current| current.expose_secret().is_empty())
    {
        *params_api_key = Some(key.clone());
    }
    validate_environment(auth, params_api_key.as_ref())
}

// map_openai_params copies every supplied key, and drop_params changes nothing
#[must_use]
pub fn video_map_params(options: &VideoCreate) -> VideoOptions {
    let mut mapped = VideoOptions::new();
    map_params(options, &mut mapped);
    mapped
}
