use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::Object;

use crate::endpoint_target::target::{
    EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::gemini_http::error::GeminiHttpError;
use crate::gemini_http::headers::psc_endpoint_format;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::{ApiVersion, UrlMode};
use crate::token_cache::probe::AccessToken;

use super::GeminiCall;

// LiteLLM's _get_token_and_url call: gemini is Google AI Studio and every other provider is Vertex
pub(super) fn resolve_target(
    call: &GeminiCall<'_>,
    optional_params: &Object,
    catalog: &ModelCatalog,
    access: &AccessToken,
    stream: bool,
) -> Result<EndpointTarget, GeminiHttpError> {
    let provider = match call.provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            TargetProvider::Vertex
        }
        VertexCapabilityProvider::Gemini => TargetProvider::Gemini,
    };
    get_token_and_url(
        catalog,
        &TokenUrlInput {
            provider,
            model: call.model,
            auth_token: Some(&access.token),
            gemini_api_key: call.gemini_api_key,
            project: Some(access.project.as_str()),
            location: call.location,
            stream,
            api_base: call.api_base,
            api_version: ApiVersion::V1,
            mode: UrlMode::Chat,
            use_psc_endpoint_format: psc_endpoint_format(optional_params)?,
        },
    )
    .map_err(|source| GeminiHttpError::Target { source })
}
