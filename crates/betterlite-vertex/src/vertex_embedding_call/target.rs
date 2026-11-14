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
pub const TEXT_CONTENT_TYPE: &str = "application/json";
pub const MULTIMODAL_CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub struct TargetRequest<'call> {
    pub provider: VertexCapabilityProvider,
    pub model: &'call str,
    pub auth_token: Option<&'call SecretString>,
    pub gemini_api_key: Option<&'call SecretString>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub api_base: Option<&'call str>,
    pub use_psc_endpoint_format: bool,
}

// LiteLLM's _get_token_and_url for an embedding: mode="embedding", version v1, and no alt=sse query
#[must_use]
pub fn embedding_target(
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

// The token becomes a bearer header and a Gemini key its own header; either way the caller's headers are written last
#[must_use]
pub fn headers(
    auth: Option<&AuthHeader>,
    content_type: &'static str,
    caller: &BTreeMap<String, String>,
) -> BTreeMap<String, SecretString> {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("Content-Type"),
        SecretString::from(content_type),
    );
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
    for (name, value) in caller {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers
}
