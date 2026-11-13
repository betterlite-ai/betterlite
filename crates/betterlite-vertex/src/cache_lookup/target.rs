use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{
    AuthHeader, CustomProxyInput, EndpointTarget, TargetProvider, check_custom_proxy,
};
use crate::model_urls::ApiVersion;
use crate::model_urls::base_url::get_vertex_base_url;

use super::error::CacheLookupError;

pub const CACHE_ENDPOINT: &str = "cachedContents";

const GEMINI_CACHE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/cachedContents";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheProvider {
    Vertex,
    VertexBeta,
    Gemini,
}

pub struct CacheTargetInput<'text> {
    pub provider: CacheProvider,
    pub api_base: Option<&'text str>,
    pub project: Option<&'text str>,
    pub location: Option<&'text str>,
    pub auth_token: Option<&'text SecretString>,
    pub gemini_api_key: Option<&'text SecretString>,
    pub model: Option<&'text str>,
}

pub fn cache_target(input: &CacheTargetInput<'_>) -> Result<EndpointTarget, CacheLookupError> {
    let version = version_for(input.provider);
    let (url, auth) = match input.provider {
        CacheProvider::Gemini => {
            let (url, auth) = gemini_target(input.gemini_api_key)?;
            (url, Some(auth))
        }
        CacheProvider::Vertex | CacheProvider::VertexBeta => (
            native_cache_url(input.provider, input.project, input.location)?,
            input.auth_token.cloned().map(AuthHeader::Token),
        ),
    };
    Ok(check_custom_proxy(&CustomProxyInput {
        api_base: input.api_base,
        provider: match input.provider {
            CacheProvider::Gemini => TargetProvider::Gemini,
            CacheProvider::Vertex | CacheProvider::VertexBeta => TargetProvider::Vertex,
        },
        gemini_api_key: input.gemini_api_key,
        endpoint: CACHE_ENDPOINT,
        stream: false,
        auth_header: auth.as_ref(),
        url: &url,
        model: input.model,
        project: input.project,
        location: input.location,
        api_version: version,
        use_psc_endpoint_format: false,
    })?)
}

const fn version_for(provider: CacheProvider) -> ApiVersion {
    match provider {
        CacheProvider::VertexBeta => ApiVersion::V1Beta1,
        CacheProvider::Vertex | CacheProvider::Gemini => ApiVersion::V1,
    }
}

fn gemini_target(
    api_key: Option<&SecretString>,
) -> Result<(String, AuthHeader), EndpointTargetError> {
    let key = api_key.ok_or(EndpointTargetError::GeminiApiKey)?;
    let mut headers = BTreeMap::new();
    headers.insert("x-goog-api-key".to_owned(), key.clone());
    Ok((GEMINI_CACHE_URL.to_owned(), AuthHeader::Headers(headers)))
}

fn native_cache_url(
    provider: CacheProvider,
    configured_project: Option<&str>,
    configured_location: Option<&str>,
) -> Result<String, EndpointTargetError> {
    let location = configured_location.ok_or(EndpointTargetError::Location)?;
    let base = get_vertex_base_url(Some(location)).map_err(EndpointTargetError::VertexUrl)?;
    let project = configured_project.ok_or(EndpointTargetError::Project)?;
    let version = version_for(provider);
    Ok(format!(
        "{base}/{}/projects/{project}/locations/{location}/{CACHE_ENDPOINT}",
        version.as_str()
    ))
}
