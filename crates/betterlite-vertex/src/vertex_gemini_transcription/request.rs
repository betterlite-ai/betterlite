use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::audio_file_input::language::normalize_language_to_bcp47;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::DropParams;
use http::header::HeaderMap;
use secrecy::SecretString;

use crate::model_urls::Endpoint;
use crate::model_urls::base_url::{get_vertex_base_url, validate_vertex_location};
use crate::vertex_gemini_transcription::error::VertexGeminiTranscriptionError;
use crate::vertex_gemini_transcription::types::{
    AudioTranscriptionConfig, Content, GenerationConfig, InlineData, Part, Role,
    TranscriptionRequest,
};
use crate::vertex_recognition::request as shared;

pub const DEFAULT_LOCATION: &str = "global";
const VERTEX_AI_PREFIX: &str = "vertex_ai/";

// LiteLLM replaces the caller's authorization, project and content-type headers, so a caller cannot redirect the call
pub fn validate_environment(
    headers: HeaderMap,
    access_token: &SecretString,
    project: &str,
) -> Result<HeaderMap, VertexGeminiTranscriptionError> {
    shared::validate_environment(headers, access_token, project)
        .map_err(VertexGeminiTranscriptionError::from)
}

pub fn map_openai_params(
    non_default: &Object,
    optional: &Object,
    drop_params: DropParams,
) -> Result<Object, VertexGeminiTranscriptionError> {
    shared::map_openai_params(non_default, optional, drop_params)
        .map_err(VertexGeminiTranscriptionError::from)
}

pub fn complete_url(
    api_base: Option<&str>,
    location: Option<&str>,
    project: Option<&str>,
    credential_project: Option<&str>,
    model: &str,
) -> Result<String, VertexGeminiTranscriptionError> {
    let checked_location = validate_vertex_location(Some(location.unwrap_or(DEFAULT_LOCATION)))
        .map_err(|source| VertexGeminiTranscriptionError::Location { source })?;
    let Some(requested) = project.or(credential_project) else {
        return Err(VertexGeminiTranscriptionError::ProjectRequired);
    };
    let project_id = shared::validate_speech_project(requested)?;
    let base = match api_base {
        Some(base) => base.to_owned(),
        None => get_vertex_base_url(Some(checked_location))
            .map_err(|source| VertexGeminiTranscriptionError::Location { source })?,
    };
    let trimmed_base = base.trim_end_matches('/');
    // the publisher route names the model without the provider prefix, which LiteLLM removes once
    let bare_model = model.strip_prefix(VERTEX_AI_PREFIX).unwrap_or(model);
    let endpoint = Endpoint::GenerateContent.as_str();
    Ok(format!(
        "{trimmed_base}/v1/projects/{project_id}/locations/{checked_location}/publishers/google/models/{bare_model}:{endpoint}"
    ))
}

#[must_use]
pub fn transcription_request(
    audio: &ProcessedAudioFile,
    language: Option<&Json>,
) -> TranscriptionRequest {
    let language_codes = match language {
        Some(Json::Text(text)) if !text.is_empty() => vec![normalize_language_to_bcp47(text)],
        _ => Vec::new(),
    };
    TranscriptionRequest {
        contents: vec![Content {
            role: Role::User,
            parts: vec![Part {
                inline_data: InlineData {
                    mime_type: audio.content_type.clone(),
                    data: STANDARD.encode(&audio.file_content),
                },
            }],
        }],
        generation_config: GenerationConfig {
            audio_transcription_config: AudioTranscriptionConfig { language_codes },
        },
    }
}
