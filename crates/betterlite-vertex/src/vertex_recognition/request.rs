use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::audio_file_input::language::normalize_language_to_bcp47;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::DropParams;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::model_urls::base_url::validate_vertex_location;
use crate::vertex_recognition::error::VertexRecognitionError;
use crate::vertex_recognition::types::{RecognitionConfig, RecognitionFeatures, RecognizeRequest};

pub const DEFAULT_LOCATION: &str = "us";
pub const AUTO_LANGUAGE_CODE: &str = "auto";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["language", "response_format"];
pub const SUPPORTED_RESPONSE_FORMATS: [&str; 2] = ["json", "text"];

const URL_UNSAFE_PROJECT_CHARS: [char; 9] = ['/', '?', '#', '\\', ':', ' ', '\t', '\n', '\r'];
const VERTEX_AI_PREFIX: &str = "vertex_ai/";

pub fn validate_speech_location(location: Option<&str>) -> Result<&str, VertexRecognitionError> {
    validate_vertex_location(Some(location.unwrap_or(DEFAULT_LOCATION)))
        .map_err(|source| VertexRecognitionError::Location { source })
}

#[must_use]
pub fn speech_to_text_host(location: &str) -> String {
    if location == "global" {
        "speech.googleapis.com".to_owned()
    } else {
        format!("{location}-speech.googleapis.com")
    }
}

pub fn validate_speech_project(project: &str) -> Result<&str, VertexRecognitionError> {
    // an empty id, a parent path or any of these characters would move the request to another resource
    let unsafe_project = project.is_empty()
        || project.contains("..")
        || project
            .chars()
            .any(|character| URL_UNSAFE_PROJECT_CHARS.contains(&character));
    if unsafe_project {
        return Err(VertexRecognitionError::Project {
            project: project.to_owned(),
        });
    }
    Ok(project)
}

pub fn map_openai_params(
    non_default: &Object,
    optional: &Object,
    drop_params: DropParams,
) -> Result<Object, VertexRecognitionError> {
    let mut mapped = optional.clone();
    for name in SUPPORTED_OPENAI_PARAMS {
        if let Some(value) = non_default.get(name) {
            mapped.insert(name.to_owned(), value.clone());
        }
    }
    let Some(format) = mapped.get("response_format") else {
        return Ok(mapped);
    };
    let accepted = format
        .as_str()
        .is_some_and(|text| SUPPORTED_RESPONSE_FORMATS.contains(&text));
    if format.is_null() || accepted {
        return Ok(mapped);
    }
    if drop_params.global || drop_params.request {
        mapped.remove("response_format");
        return Ok(mapped);
    }
    Err(VertexRecognitionError::ResponseFormat {
        format: format.to_string(),
    })
}

// LiteLLM replaces these three headers after the caller's own, so a caller cannot point the request elsewhere
pub fn validate_environment(
    mut headers: HeaderMap,
    access_token: &SecretString,
    project: &str,
) -> Result<HeaderMap, VertexRecognitionError> {
    let authorization = HeaderValue::try_from(format!("Bearer {}", access_token.expose_secret()))
        .map_err(|source| VertexRecognitionError::Authorization { source })?;
    headers.insert(AUTHORIZATION, authorization);
    let user_project = HeaderValue::try_from(project)
        .map_err(|source| VertexRecognitionError::UserProject { source })?;
    headers.insert(HeaderName::from_static("x-goog-user-project"), user_project);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

pub fn complete_url(
    api_base: Option<&str>,
    location: Option<&str>,
    project: Option<&str>,
    credential_project: Option<&str>,
) -> Result<String, VertexRecognitionError> {
    let checked_location = validate_speech_location(location)?;
    let Some(requested) = project.or(credential_project) else {
        return Err(VertexRecognitionError::ProjectRequired);
    };
    let project_id = validate_speech_project(requested)?;
    let base = api_base.map_or_else(
        || format!("https://{}", speech_to_text_host(checked_location)),
        str::to_owned,
    );
    let trimmed_base = base.trim_end_matches('/');
    Ok(format!(
        "{trimmed_base}/v2/projects/{project_id}/locations/{checked_location}/recognizers/_:recognize"
    ))
}

#[must_use]
pub fn recognize_request(
    model: &str,
    audio: &ProcessedAudioFile,
    language: Option<&Json>,
) -> RecognizeRequest {
    let language_code = match language {
        Some(Json::Text(text)) if !text.is_empty() => normalize_language_to_bcp47(text),
        _ => AUTO_LANGUAGE_CODE.to_owned(),
    };
    RecognizeRequest {
        config: RecognitionConfig {
            model: model
                .strip_prefix(VERTEX_AI_PREFIX)
                .unwrap_or(model)
                .to_owned(),
            language_codes: vec![language_code],
            features: RecognitionFeatures {
                enable_automatic_punctuation: true,
            },
            auto_decoding_config: Object::new(),
        },
        content: STANDARD.encode(&audio.file_content),
    }
}
