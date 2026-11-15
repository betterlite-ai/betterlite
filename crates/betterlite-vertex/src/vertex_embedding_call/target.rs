use std::collections::BTreeMap;

use betterlite_models::catalog::ModelCatalog;
use secrecy::{ExposeSecret as _, SecretString};

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::{
    AuthHeader, EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::{ApiVersion, UrlMode};

// VertexBase.set_headers writes this content type; the multimodal validate_environment forces the charset too
const TEXT_CONTENT_TYPE: &str = "application/json";
const MULTIMODAL_CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub(super) struct TargetRequest<'call> {
    pub(super) provider: VertexCapabilityProvider,
    pub(super) model: &'call str,
    pub(super) auth_token: Option<&'call SecretString>,
    pub(super) gemini_api_key: Option<&'call SecretString>,
    pub(super) project: Option<&'call str>,
    pub(super) location: Option<&'call str>,
    pub(super) api_base: Option<&'call str>,
    pub(super) use_psc_endpoint_format: bool,
}

// LiteLLM's _get_token_and_url for an embedding: mode="embedding", version v1, and no alt=sse query
pub(super) fn embedding_target(
    catalog: &ModelCatalog,
    request: &TargetRequest<'_>,
) -> Result<EndpointTarget, EndpointTargetError> {
    get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider: target_provider(request.provider),
            model: request.model,
            auth_token: request.auth_token,
            gemini_api_key: request.gemini_api_key,
            project: request.project,
            location: request.location,
            stream: false,
            api_base: request.api_base,
            api_version: ApiVersion::V1,
            mode: UrlMode::Embedding,
            use_psc_endpoint_format: request.use_psc_endpoint_format,
        },
    )
}

const fn target_provider(provider: VertexCapabilityProvider) -> TargetProvider {
    match provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            TargetProvider::Vertex
        }
        VertexCapabilityProvider::Gemini => TargetProvider::Gemini,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HeaderMode {
    // VertexBase.set_headers writes its own headers first and the caller's after, so a caller header wins
    Text,
    // The multimodal validate_environment updates the caller's dict, so the forced content type and auth win
    Multimodal,
}

#[must_use]
pub(super) fn embedding_headers(
    auth: Option<&AuthHeader>,
    caller: &BTreeMap<String, String>,
    mode: HeaderMode,
) -> BTreeMap<String, SecretString> {
    let content_type = match mode {
        HeaderMode::Text => TEXT_CONTENT_TYPE,
        HeaderMode::Multimodal => MULTIMODAL_CONTENT_TYPE,
    };
    let forced = forced_headers(auth, content_type);
    if matches!(mode, HeaderMode::Multimodal) {
        let mut kept: BTreeMap<String, SecretString> = caller
            .iter()
            .filter(|(name, _)| {
                !forced
                    .keys()
                    .any(|forced_name| forced_name.eq_ignore_ascii_case(name.as_str()))
            })
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect();
        kept.extend(forced);
        return kept;
    }
    let mut headers = forced;
    for (name, value) in caller {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers
}

fn forced_headers(
    auth: Option<&AuthHeader>,
    content_type: &'static str,
) -> BTreeMap<String, SecretString> {
    let mut headers = BTreeMap::from([(
        String::from("Content-Type"),
        SecretString::from(content_type),
    )]);
    match auth {
        Some(AuthHeader::Token(token)) => {
            headers.insert(
                String::from("Authorization"),
                SecretString::from(format!("Bearer {}", token.expose_secret())),
            );
        }
        Some(AuthHeader::Headers(values)) => {
            for (name, value) in values {
                headers.insert(name.clone(), value.clone());
            }
        }
        None => {}
    }
    headers
}
