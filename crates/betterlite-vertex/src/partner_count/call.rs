use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::credential_sources::getters::VertexParams;
use crate::model_policy::error::VertexAiError;
use crate::vertex_interactions::mint::MintAccessToken;

use super::body::PartnerCountBody;
use super::execute::{PartnerCountCall, PartnerCountTransport, handle_count_tokens_request};
use super::options::PartnerCountOptions;
use super::response::PartnerCount;

// the explicit Vertex settings LiteLLM overlays onto a copy of the deployment params
pub struct PartnerCountOverrides<'call> {
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
}

pub struct PartnerCountRequest<'call> {
    pub model: &'call str,
    pub messages: &'call [Json],
    pub params: VertexParams<'call>,
    pub options: &'call PartnerCountOptions,
    pub settings: &'call VertexSettings,
    pub api_base: Option<&'call str>,
    pub overrides: PartnerCountOverrides<'call>,
}

// LiteLLM's VertexAIPartnerModels.count_tokens: the body carries only model and messages, a truthy explicit setting overwrites the deployment copy, and only a status-bearing failure keeps its status
pub async fn count_tokens<T>(
    transport: &T,
    request: &PartnerCountRequest<'_>,
    mint: &dyn MintAccessToken,
) -> Result<PartnerCount, VertexAiError>
where
    T: PartnerCountTransport,
{
    let body = PartnerCountBody {
        model: Some(request.model.to_owned()),
        messages: Some(request.messages.to_vec()),
        extra: IndexMap::new(),
    };
    handle_count_tokens_request(
        transport,
        &PartnerCountCall {
            model: request.model,
            body: &body,
            api_base: request.api_base,
            params: VertexParams {
                project: request
                    .overrides
                    .project
                    .filter(|value| !value.is_empty())
                    .or(request.params.project),
                ai_project: request.params.ai_project,
                location: request
                    .overrides
                    .location
                    .filter(|value| !value.is_empty())
                    .or(request.params.location),
                ai_location: request.params.ai_location,
                credentials: request.overrides.credentials.or(request.params.credentials),
                ai_credentials: request.params.ai_credentials,
            },
            options: request.options,
            settings: request.settings,
        },
        mint,
    )
    .await
    .map_err(|error| VertexAiError {
        status_code: error.status_code().unwrap_or(500),
        message: error.to_string(),
        headers: None,
    })
}
