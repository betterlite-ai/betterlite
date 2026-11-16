use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use super::error::VeoRequestError;

pub const DEFAULT_VIDEO_MIME: &str = "video/mp4";

// _VeoOperation, reduced to the fields an edit reads from the prefetched source
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PrefetchedOperation {
    pub done: Option<bool>,
    pub response: Option<OperationResponse>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OperationResponse {
    pub videos: Option<Vec<SourceVideo>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SourceVideo {
    #[serde(rename = "gcsUri")]
    pub gcs_uri: Option<String>,
    #[serde(rename = "bytesBase64Encoded")]
    pub bytes_base64_encoded: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
}

pub struct EditInput<'input> {
    pub prompt: &'input str,
    pub video_id: &'input str,
    pub api_base: &'input str,
    pub extra_body: Option<&'input Object>,
    pub prefetched: Option<&'input PrefetchedOperation>,
}

#[derive(Debug)]
pub struct EditForm {
    pub url: String,
    pub body: Object,
}

pub fn edit_request(input: &EditInput<'_>) -> Result<EditForm, VeoRequestError> {
    let prefetched = input.prefetched.ok_or(VeoRequestError::PrefetchRequired)?;
    if !prefetched.done.unwrap_or(false) {
        return Err(VeoRequestError::SourceNotDone);
    }
    let videos = prefetched
        .response
        .as_ref()
        .and_then(|response| response.videos.as_deref())
        .unwrap_or_default();
    let source = videos.first().ok_or(VeoRequestError::NoVideos)?;
    let operation = decode_video_id_with_provider(input.video_id);
    // the model is element seven, present only in a name of eight or more components
    let model = operation.video_id.split('/').nth(7).unwrap_or_default();
    let mut instance = Object::new();
    instance.insert("prompt".to_owned(), Json::Text(input.prompt.to_owned()));
    instance.insert("video".to_owned(), Json::Object(source_video(source)?));
    let mut body = Object::new();
    body.insert(
        "instances".to_owned(),
        Json::List(vec![Json::Object(instance)]),
    );
    if let Some(parameters) = edit_parameters(input.extra_body) {
        body.insert("parameters".to_owned(), Json::Object(parameters));
    }
    Ok(EditForm {
        url: format!(
            "{}/{model}:predictLongRunning",
            input.api_base.trim_end_matches('/')
        ),
        body,
    })
}

// the source's gcsUri wins by key presence even when its value is empty; base64 falls back to mp4
fn source_video(source: &SourceVideo) -> Result<Object, VeoRequestError> {
    let mut video = Object::new();
    if let Some(uri) = &source.gcs_uri {
        video.insert("gcsUri".to_owned(), Json::Text(uri.clone()));
        return Ok(video);
    }
    let Some(encoded) = &source.bytes_base64_encoded else {
        return Err(VeoRequestError::MissingSourceVideo);
    };
    video.insert("bytesBase64Encoded".to_owned(), Json::Text(encoded.clone()));
    let mime_type = source.mime_type.as_deref().unwrap_or(DEFAULT_VIDEO_MIME);
    video.insert("mimeType".to_owned(), Json::Text(mime_type.to_owned()));
    Ok(video)
}

// extra_body's nested parameters are flattened first, then its flat options override them
fn edit_parameters(extra_body: Option<&Object>) -> Option<Object> {
    let extra = extra_body?;
    let mut parameters = match extra.get("parameters") {
        Some(Json::Object(nested)) => nested.clone(),
        _ => Object::new(),
    };
    for (name, value) in extra {
        if name != "parameters" {
            parameters.insert(name.clone(), value.clone());
        }
    }
    (!parameters.is_empty()).then_some(parameters)
}
