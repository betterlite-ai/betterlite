use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::url_path::encode_segment;
use betterlite_models::metadata::vendor::VertexAudioApi;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::endpoint_target::target::get_api_base;
use crate::model_policy::lyria::LyriaOverlay;
use crate::studio_interactions::credentials::RequestHeaders;
use crate::vertex_interactions::DEFAULT_LOCATION as INTERACTIONS_LOCATION;
use crate::vertex_interactions::config::{InteractionCall, VertexInteractionsConfig};
use crate::vertex_interactions::mint::{MintAccessToken, ProjectOnlyMint};
use crate::vertex_speech::request::validate_environment;

use super::error::VertexLyriaError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LyriaPrompt {
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LyriaPredictParameters {
    pub sample_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LyriaPredictBody {
    pub instances: Vec<LyriaPrompt>,
    pub parameters: LyriaPredictParameters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LyriaAudioFormat {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub mime_type: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LyriaInteractionsBody {
    pub model: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<LyriaAudioFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LyriaRequestBody {
    Predict(LyriaPredictBody),
    Interactions(LyriaInteractionsBody),
}

#[derive(Debug, Clone, Copy)]
pub struct LyriaCall<'call> {
    pub model: &'call str,
    pub api_base: Option<&'call str>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub input: &'call str,
    pub optional_params: &'call Object,
}

#[derive(Debug)]
pub struct LyriaRequest {
    pub url: String,
    pub body: LyriaRequestBody,
    pub headers: RequestHeaders,
}

pub async fn prepare_request(
    mint: &dyn MintAccessToken,
    overlay: &LyriaOverlay<'_>,
    call: &LyriaCall<'_>,
    caller_headers: &IndexMap<String, String>,
) -> Result<LyriaRequest, VertexLyriaError> {
    let model_info =
        overlay
            .model_info(call.model)
            .ok_or_else(|| VertexLyriaError::UnknownModel {
                model: call.model.to_owned(),
            })?;
    let configured_project = call.project.filter(|value| !value.is_empty());
    let access = mint
        .mint(call.credentials, configured_project)
        .await
        .map_err(VertexLyriaError::Mint)?;
    let project = configured_project.unwrap_or(access.project.as_str());
    let url = match model_info.vertex_ai_audio_api {
        VertexAudioApi::LyriaPredict => {
            predict_url_at(call.api_base, project, location(call), call.model)?
        }
        VertexAudioApi::LyriaInteractions => interactions_url(call, project).await?,
    };
    let mut headers = validate_environment(caller_headers);
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", access.token.expose_secret())),
    );
    headers.insert(
        "x-goog-user-project".to_owned(),
        SecretString::from(access.project.clone()),
    );
    Ok(LyriaRequest {
        url,
        body: request_body(call, model_info.vertex_ai_audio_api),
        headers,
    })
}

fn location(call: &LyriaCall<'_>) -> &str {
    call.location
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LOCATION)
}

fn predict_url_at(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    model: &str,
) -> Result<String, VertexLyriaError> {
    let base = get_api_base(api_base, Some(location)).map_err(VertexLyriaError::ApiBase)?;
    let base_model = model.strip_prefix("vertex_ai/").unwrap_or(model);
    let project =
        encode_segment(project, "project").map_err(|source| VertexLyriaError::Segment {
            field: "project",
            source,
        })?;
    let location =
        encode_segment(location, "location").map_err(|source| VertexLyriaError::Segment {
            field: "location",
            source,
        })?;
    let model =
        encode_segment(base_model, "model").map_err(|source| VertexLyriaError::Segment {
            field: "model",
            source,
        })?;
    Ok(format!(
        "{}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:predict",
        base.trim_end_matches('/')
    ))
}

// LiteLLM builds this URL through the interactions config, whose injected mint only resolves the project
async fn interactions_url(call: &LyriaCall<'_>, project: &str) -> Result<String, VertexLyriaError> {
    let settings = VertexSettings::default();
    let params = VertexParams {
        project: Some(project),
        ai_project: None,
        location: Some(INTERACTIONS_LOCATION),
        ai_location: None,
        credentials: call.credentials,
        ai_credentials: None,
    };
    let config = VertexInteractionsConfig {
        mint: Arc::new(ProjectOnlyMint {
            project: project.to_owned(),
        }),
    };
    let interaction = InteractionCall {
        api_base: call.api_base,
        params,
        settings: &settings,
    };
    config
        .get_complete_url(&interaction, false)
        .await
        .map_err(VertexLyriaError::Interactions)
}

fn request_body(call: &LyriaCall<'_>, api: VertexAudioApi) -> LyriaRequestBody {
    let model = call.model.strip_prefix("vertex_ai/").unwrap_or(call.model);
    let format = call
        .optional_params
        .get("response_format")
        .and_then(Json::as_str);
    match api {
        VertexAudioApi::LyriaPredict => LyriaRequestBody::Predict(LyriaPredictBody {
            instances: vec![LyriaPrompt {
                prompt: call.input.to_owned(),
            }],
            parameters: LyriaPredictParameters { sample_count: 1 },
        }),
        VertexAudioApi::LyriaInteractions => {
            LyriaRequestBody::Interactions(LyriaInteractionsBody {
                model: model.to_owned(),
                input: call.input.to_owned(),
                response_format: (format == Some("wav")).then_some(LyriaAudioFormat {
                    kind: "audio",
                    mime_type: "audio/wav",
                }),
            })
        }
    }
}
