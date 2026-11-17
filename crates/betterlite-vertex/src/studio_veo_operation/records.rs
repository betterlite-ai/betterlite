use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use betterlite_prompts::arguments::json::{Json, Object};

use super::decode::decode_studio_veo_operation;
use super::error::StudioVeoOperationError;
use crate::vertex_veo_operation::decode::decoded_json;
use crate::vertex_veo_operation::usage::video_usage;

// the caller-side inputs a processing response reads: whose provider it is, the options it asked for and the loaded settings
pub struct VideoContext<'call> {
    pub provider: Option<&'call str>,
    pub request_data: Option<&'call Object>,
    pub settings: &'call VertexSettings,
}

// transform_video_create_response: the operation name is the polling id, the caller's model stays and usage comes from the request
pub fn create_response(
    model: &str,
    body: &[u8],
    context: &VideoContext<'_>,
) -> Result<VideoObject, StudioVeoOperationError> {
    let operation = decode_studio_veo_operation(body)?;
    if operation.name.is_empty() {
        return Err(StudioVeoOperationError::MissingName);
    }
    let mut usage = video_usage(context.request_data, context.settings)?;
    if let Some(empty) = retained_empty_resolution(context.request_data) {
        usage.insert("video_resolution".to_owned(), Json::Text(empty));
    }
    Ok(VideoObject {
        id: encode_video_id_with_provider(
            &operation.name,
            context.provider.unwrap_or_default(),
            Some(model),
        ),
        object: VideoObjectType::Video,
        status: "processing".to_owned(),
        created_at: None,
        completed_at: None,
        expires_at: None,
        error: None,
        progress: None,
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: Some(model.to_owned()),
        usage: Some(decoded_json(usage)?),
    })
}

// transform_video_status_retrieve_response: done alone decides the status, and the polling id carries no model
pub fn status_response(
    body: &[u8],
    provider: Option<&str>,
) -> Result<VideoObject, StudioVeoOperationError> {
    let operation = decode_studio_veo_operation(body)?;
    let status = if operation.done {
        "completed"
    } else {
        "processing"
    };
    Ok(VideoObject {
        id: encode_video_id_with_provider(&operation.name, provider.unwrap_or_default(), None),
        object: VideoObjectType::Video,
        status: status.to_owned(),
        created_at: None,
        completed_at: None,
        expires_at: None,
        error: None,
        progress: None,
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: None,
        usage: None,
    })
}

// transform_video_status_retrieve_request: the polling URL unwraps the encoded video id and sends no query params
#[must_use]
pub fn status_request(video_id: &str, api_base: &str) -> StatusRequest {
    let operation = decode_video_id_with_provider(video_id);
    StatusRequest {
        url: format!(
            "{}/v1beta/{}",
            api_base.trim_end_matches('/'),
            operation.video_id
        ),
        params: Object::new(),
    }
}

pub struct StatusRequest {
    pub url: String,
    pub params: Object,
}

// _usage_video_resolution_from_parameters: LiteLLM tests emptiness before stripping, so a whitespace-only resolution survives as ""
fn retained_empty_resolution(request_data: Option<&Object>) -> Option<String> {
    let parameters = request_data?.get("parameters")?.as_object()?;
    let Json::Text(resolution) = parameters.get("resolution")? else {
        return None;
    };
    if resolution.is_empty() || !resolution.trim().is_empty() {
        return None;
    }
    Some(String::new())
}

mod tests;
