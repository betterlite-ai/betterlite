use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;
use url::Url;

use super::claude_mapping::{self, ClaudeMappingError, ClaudeMessage};
use super::event::SuccessEvent;
use super::initialization::Settings;
use super::metadata_cleanup;
use super::model_fallback::{self, Provider, ResolvedModel};
use super::routing::{self, RoutingError};
use super::timing::{self, Timing, TimingError};

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("the Claude mapping failed")]
    Claude(#[source] Box<ClaudeMappingError>),
    #[error("the log endpoint could not be chosen")]
    Routing(#[source] Box<RoutingError>),
    #[error("the timing envelope could not be built")]
    Timing(#[source] Box<TimingError>),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub provider_request: ProviderRequest,
    pub provider_response: ProviderResponse,
    pub timing: Timing,
}

// No Debug: the meta map carries the Helicone bearer token that assembly wrote.
#[derive(Serialize)]
pub struct ProviderRequest {
    pub url: &'static str,
    pub json: RequestJson,
    pub meta: IndexMap<String, Json>,
}

#[derive(Debug, Serialize)]
pub struct RequestJson {
    pub model: String,
    pub messages: Vec<Json>,
}

#[derive(Debug, Serialize)]
pub struct ProviderResponse {
    pub json: ResponseJson,
    pub headers: ResponseHeaders,
    pub status: u16,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseJson {
    Claude(ClaudeMessage),
    Passthrough(Json),
}

impl ResponseJson {
    fn build(model: &ResolvedModel, response: &Json) -> Result<Self, EnvelopeError> {
        match model.provider {
            Provider::Claude => {
                let mapped = claude_mapping::map(&model.name, response)
                    .map_err(|error| EnvelopeError::Claude(Box::new(error)))?;
                Ok(Self::Claude(mapped))
            }
            Provider::OpenAi | Provider::VertexAi | Provider::Gemini => {
                Ok(Self::Passthrough(response.clone()))
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ResponseHeaders {
    #[serde(rename = "openai-version")]
    pub openai_version: &'static str,
}

// No Debug: the envelope's meta carries the Helicone bearer token.
pub struct Assembled {
    pub log_url: Url,
    pub envelope: Envelope,
}

pub fn assemble(
    settings: &Settings,
    key: &SecretString,
    event: &SuccessEvent,
) -> Result<Assembled, EnvelopeError> {
    let metadata = metadata_cleanup::resolve(&event.litellm_params);
    let model = model_fallback::resolve(&event.model, &event.litellm_params.custom_llm_provider);
    let response = ResponseJson::build(&model, &event.response)?;
    let destination = routing::destination(&settings.api_base, &model)
        .map_err(|error| EnvelopeError::Routing(Box::new(error)))?;
    let timing = Timing {
        start_time: timing::moment(event.start_time)
            .map_err(|error| EnvelopeError::Timing(Box::new(error)))?,
        end_time: timing::moment(event.end_time)
            .map_err(|error| EnvelopeError::Timing(Box::new(error)))?,
    };
    // The metadata is applied last, so a metadata key can replace Helicone-Auth.
    let mut meta = IndexMap::new();
    meta.insert(
        "Helicone-Auth".to_owned(),
        Json::Text(format!("Bearer {}", key.expose_secret())),
    );
    meta.extend(metadata);
    Ok(Assembled {
        log_url: destination.log_url,
        envelope: Envelope {
            provider_request: ProviderRequest {
                url: destination.provider_url,
                json: RequestJson {
                    model: model.name,
                    messages: event.messages.clone(),
                },
                meta,
            },
            provider_response: ProviderResponse {
                json: response,
                headers: ResponseHeaders {
                    openai_version: "2020-10-01",
                },
                status: 200,
            },
            timing,
        },
    })
}

mod tests;
