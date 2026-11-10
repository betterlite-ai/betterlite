use std::collections::BTreeMap;

use betterlite_models::catalog::ModelCatalog;
use secrecy::SecretString;
use url::Url;

use crate::model_urls::base_url::get_vertex_base_url;
use crate::model_urls::gemini::{GeminiUrlInput, get_gemini_url};
use crate::model_urls::model_name::get_vertex_base_model_name;
use crate::model_urls::vertex::{VertexUrlInput, get_vertex_url};
use crate::model_urls::{ApiVersion, UrlMode, VertexUrl};

use super::error::EndpointTargetError;
use super::graft::{graft_default_vertex_path, parse_base};
use super::partner::{Partner, PartnerUrlInput, create_vertex_url};
use super::region::{DEFAULT_LOCATION, select_region};

#[derive(Debug, Clone)]
pub enum AuthHeader {
    Token(SecretString),
    Headers(BTreeMap<String, SecretString>),
}

#[derive(Debug, Clone)]
pub struct EndpointTarget {
    pub url: String,
    pub auth: Option<AuthHeader>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetProvider {
    Gemini,
    Vertex,
}

pub struct CustomProxyInput<'text> {
    pub api_base: Option<&'text str>,
    pub provider: TargetProvider,
    pub gemini_api_key: Option<&'text SecretString>,
    pub endpoint: &'text str,
    pub stream: bool,
    pub auth_header: Option<&'text AuthHeader>,
    pub url: &'text str,
    pub model: Option<&'text str>,
    pub project: Option<&'text str>,
    pub location: Option<&'text str>,
    pub api_version: ApiVersion,
    pub use_psc_endpoint_format: bool,
}

pub fn get_api_base(
    api_base: Option<&str>,
    location: Option<&str>,
) -> Result<String, EndpointTargetError> {
    if let Some(base) = api_base.filter(|base| !base.is_empty()) {
        return Ok(base.to_owned());
    }
    // LiteLLM reads an empty location as absent and falls back to the default region
    let fallback = location
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LOCATION);
    Ok(get_vertex_base_url(Some(fallback))?)
}

pub fn check_custom_proxy(
    input: &CustomProxyInput<'_>,
) -> Result<EndpointTarget, EndpointTargetError> {
    let mut auth = input.auth_header.cloned();
    let mut url = input.url.to_owned();
    let Some(api_base) = input.api_base.filter(|base| !base.is_empty()) else {
        return Ok(EndpointTarget { url, auth });
    };
    if input.provider == TargetProvider::Gemini {
        let model = input.model.ok_or(EndpointTargetError::GeminiModel)?;
        url = format!("{api_base}/models/{model}:{}", input.endpoint);
        let key = input
            .gemini_api_key
            .ok_or(EndpointTargetError::GeminiApiKey)?;
        let mut headers = BTreeMap::new();
        headers.insert("x-goog-api-key".to_owned(), key.clone());
        auth = Some(AuthHeader::Headers(headers));
    } else if input.use_psc_endpoint_format {
        url = psc_endpoint_url(api_base, input)?;
    } else {
        url = vertex_proxy_url(api_base, input.url, input.endpoint)?;
    }
    if input.stream {
        url = append_alt_sse(&url);
    }
    Ok(EndpointTarget { url, auth })
}

pub struct TokenUrlInput<'text> {
    pub provider: TargetProvider,
    pub model: &'text str,
    pub auth_token: Option<&'text SecretString>,
    pub gemini_api_key: Option<&'text SecretString>,
    pub project: Option<&'text str>,
    pub location: Option<&'text str>,
    pub stream: bool,
    pub api_base: Option<&'text str>,
    pub api_version: ApiVersion,
    pub mode: UrlMode,
    pub use_psc_endpoint_format: bool,
}

type NativeSelection = (VertexUrl, Option<AuthHeader>, ApiVersion, Option<String>);

pub fn get_token_and_url(
    catalog: &ModelCatalog,
    input: &TokenUrlInput<'_>,
) -> Result<EndpointTarget, EndpointTargetError> {
    let (url, auth, version, location) = if input.provider == TargetProvider::Gemini {
        gemini_target(input)?
    } else {
        vertex_target(catalog, input)?
    };
    check_custom_proxy(&CustomProxyInput {
        api_base: input.api_base,
        provider: input.provider,
        gemini_api_key: input.gemini_api_key,
        endpoint: url.endpoint.as_str(),
        stream: input.stream,
        auth_header: auth.as_ref(),
        url: &url.url,
        model: Some(input.model),
        project: input.project,
        location: location.as_deref(),
        api_version: version,
        use_psc_endpoint_format: input.use_psc_endpoint_format,
    })
}

fn gemini_target(input: &TokenUrlInput<'_>) -> Result<NativeSelection, EndpointTargetError> {
    let key = input
        .gemini_api_key
        .ok_or(EndpointTargetError::GeminiApiKey)?;
    let url = get_gemini_url(&GeminiUrlInput {
        mode: input.mode,
        model: input.model,
        stream: Some(input.stream),
    })?;
    let mut headers = BTreeMap::new();
    headers.insert("x-goog-api-key".to_owned(), key.clone());
    Ok((
        url,
        Some(AuthHeader::Headers(headers)),
        ApiVersion::V1,
        input.location.map(str::to_owned),
    ))
}

fn vertex_target(
    catalog: &ModelCatalog,
    input: &TokenUrlInput<'_>,
) -> Result<NativeSelection, EndpointTargetError> {
    let region = select_region(catalog, input.model, input.location).region;
    let version = input.api_version;
    let url = get_vertex_url(
        catalog,
        &VertexUrlInput {
            mode: input.mode,
            model: input.model,
            stream: Some(input.stream),
            project: input.project,
            location: Some(&region),
            api_version: version,
        },
    )?;
    let auth = input.auth_token.cloned().map(AuthHeader::Token);
    Ok((url, auth, version, Some(region)))
}

pub struct PartnerRequest<'text> {
    pub custom_api_base: Option<&'text str>,
    pub location: Option<&'text str>,
    pub vertex_project: Option<&'text str>,
    pub project_id: &'text str,
    pub partner: Partner,
    pub stream: bool,
    pub model: &'text str,
}

pub fn get_complete_vertex_url(
    catalog: &ModelCatalog,
    request: &PartnerRequest<'_>,
) -> Result<String, EndpointTargetError> {
    let region = select_region(catalog, request.model, request.location).region;
    let api_base = get_api_base(request.custom_api_base, Some(&region))?;
    let project = request
        .vertex_project
        .filter(|value| !value.is_empty())
        .unwrap_or(request.project_id);
    if project.is_empty() {
        return Err(EndpointTargetError::Project);
    }
    let default_url = create_vertex_url(&PartnerUrlInput {
        location: &region,
        project,
        partner: request.partner,
        stream: request.stream,
        model: request.model,
        api_base: Some(&api_base),
    })?;
    if request.partner == Partner::Llama {
        return Ok(default_url);
    }
    let endpoint = default_url
        .rsplit_once(':')
        .map_or("", |(_head, endpoint)| endpoint);
    let target = check_custom_proxy(&CustomProxyInput {
        api_base: request.custom_api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint,
        stream: request.stream,
        auth_header: None,
        url: &default_url,
        model: Some(request.model),
        project: Some(project),
        location: Some(&region),
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

fn psc_endpoint_url(
    api_base: &str,
    input: &CustomProxyInput<'_>,
) -> Result<String, EndpointTargetError> {
    let project = input
        .project
        .filter(|value| !value.is_empty())
        .ok_or(EndpointTargetError::PscInputs)?;
    let location = input
        .location
        .filter(|value| !value.is_empty())
        .ok_or(EndpointTargetError::PscInputs)?;
    let model = input
        .model
        .filter(|value| !value.is_empty())
        .ok_or(EndpointTargetError::PscInputs)?;
    let base_model = get_vertex_base_model_name(model);
    let version = input.api_version;
    Ok(format!(
        "{}/{}/projects/{project}/locations/{location}/endpoints/{base_model}:{}",
        api_base.trim_end_matches('/'),
        version.as_str(),
        input.endpoint
    ))
}

fn vertex_proxy_url(
    api_base: &str,
    default_url: &str,
    endpoint: &str,
) -> Result<String, EndpointTargetError> {
    let base = parse_base(api_base)?;
    let default =
        Url::parse(default_url).map_err(|source| EndpointTargetError::DefaultUrl { source })?;
    if matches!(base.path(), "" | "/") {
        // LiteLLM appends the default path to the text, so a host-only base keeps every base byte
        let mut url = api_base.trim_end_matches('/').to_owned();
        url.push_str(default.path());
        return Ok(url);
    }
    let version_root = matches!(base.path().trim_end_matches('/'), "/v1" | "/v1beta1");
    if version_root && default.path().contains("/projects/") {
        return graft_default_vertex_path(api_base, default_url);
    }
    Ok(format!("{api_base}:{endpoint}"))
}

fn append_alt_sse(url: &str) -> String {
    let (before_fragment, fragment) = url
        .split_once('#')
        .map_or((url, None), |(before, fragment)| (before, Some(fragment)));
    let updated = match before_fragment.split_once('?') {
        Some((path, query)) if !query.is_empty() => format!("{path}?{query}&alt=sse"),
        Some((path, _empty)) => format!("{path}?alt=sse"),
        None => format!("{before_fragment}?alt=sse"),
    };
    match fragment {
        Some(tail) => format!("{updated}#{tail}"),
        None => updated,
    }
}
