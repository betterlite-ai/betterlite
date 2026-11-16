use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use betterlite_prompts::arguments::json::Object;

use super::decode::{decode_vertex_veo_operation, decoded_json};
use super::error::VeoOperationError;
use super::usage::video_usage;
use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};

// transform_video_create_response: the operation name is the polling id, and the model is the one the caller asked for
pub fn create_response(
    model: &str,
    body: &[u8],
    provider: Option<&str>,
    request_data: Option<&Object>,
    settings: &VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if operation.name.is_empty() {
        return Err(VeoOperationError::MissingName);
    }
    processing_video(&operation.name, provider, model, request_data, settings)
}

// transform_video_status_retrieve_response: a truthy error wins over done, and usage is never inferred
pub fn status_response(
    body: &[u8],
    provider: Option<&str>,
    now: i64,
    local: &LocalTimeContext,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    let failed = operation
        .error
        .as_ref()
        .is_some_and(|error| !error.is_empty());
    let status = if failed {
        "failed"
    } else if operation.done {
        "completed"
    } else {
        "processing"
    };
    let model = operation.name.split('/').nth(7);
    // a missing or invalid createTime falls back to the current epoch second
    let created_at = operation
        .metadata
        .create_time
        .and_then(|text| decode_vertex_timestamp(text.as_bytes(), local).ok())
        .unwrap_or(now);
    Ok(VideoObject {
        id: encode_video_id_with_provider(&operation.name, provider.unwrap_or_default(), model),
        object: VideoObjectType::Video,
        status: status.to_owned(),
        created_at: Some(created_at),
        completed_at: None,
        expires_at: None,
        error: operation.error,
        progress: None,
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: model.map(str::to_owned),
        usage: None,
    })
}

// transform_video_edit_response: the model comes from the operation name, and the request data carries the usage
pub fn edit_response(
    body: &[u8],
    provider: Option<&str>,
    request_data: Option<&Object>,
    settings: &VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if operation.name.is_empty() {
        return Err(VeoOperationError::MissingEditName);
    }
    let model = operation.name.split('/').nth(7).unwrap_or_default();
    processing_video(&operation.name, provider, model, request_data, settings)
}

// transform_video_create_response and transform_video_edit_response both start a processing video with request usage
fn processing_video(
    operation_name: &str,
    provider: Option<&str>,
    model: &str,
    request_data: Option<&Object>,
    settings: &VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    Ok(VideoObject {
        id: encode_video_id_with_provider(
            operation_name,
            provider.unwrap_or_default(),
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
        usage: Some(decoded_json(video_usage(request_data, settings)?)?),
    })
}
