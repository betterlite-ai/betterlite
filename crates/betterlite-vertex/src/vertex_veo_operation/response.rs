use betterlite_config::vertex::video_settings::VertexSettings;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use betterlite_prompts::arguments::json::{Json, Object};

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
    settings: VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if operation.name.is_empty() {
        return Err(VeoOperationError::MissingName);
    }
    processing_video(
        &operation.name,
        provider,
        Some(model),
        request_data,
        settings,
    )
}

// transform_video_status_retrieve_response: a truthy error wins over done, and usage is never inferred
pub fn status_response(
    body: &[u8],
    provider: Option<&str>,
    now: i64,
    local: &LocalTimeContext,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    let failed = match &operation.error {
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Null) | None => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VeoOperationError::ErrorShape);
        }
    };
    let status = if failed {
        "failed"
    } else if operation.done {
        "completed"
    } else {
        "processing"
    };
    let model = operation.name.split('/').nth(7);
    let id = encode_video_id_with_provider(&operation.name, provider.unwrap_or_default(), model);
    let mut video = video_object(id, status, model.map(str::to_owned));
    video.created_at = Some(created_at(operation.metadata.as_ref(), now, local)?);
    if let Some(Json::Object(fields)) = &operation.error {
        video.error = Some(decoded_json(fields.clone())?);
    }
    Ok(video)
}

// transform_video_edit_response: the model comes from the operation name, and the request data carries the usage
pub fn edit_response(
    body: &[u8],
    provider: Option<&str>,
    request_data: Option<&Object>,
    settings: VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    let operation = decode_vertex_veo_operation(body)?;
    if operation.name.is_empty() {
        return Err(VeoOperationError::MissingEditName);
    }
    let model = operation.name.split('/').nth(7).unwrap_or_default();
    processing_video(
        &operation.name,
        provider,
        Some(model),
        request_data,
        settings,
    )
}

// transform_video_create_response and transform_video_edit_response both start a processing video with request usage
fn processing_video(
    operation_name: &str,
    provider: Option<&str>,
    model: Option<&str>,
    request_data: Option<&Object>,
    settings: VertexSettings,
) -> Result<VideoObject, VeoOperationError> {
    let id = encode_video_id_with_provider(operation_name, provider.unwrap_or_default(), model);
    let mut video = video_object(id, "processing", model.map(str::to_owned));
    video.usage = Some(decoded_json(video_usage(request_data, settings)?)?);
    Ok(video)
}

fn video_object(id: String, status: &str, model: Option<String>) -> VideoObject {
    VideoObject {
        id,
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
        model,
        usage: None,
    }
}

// metadata.get("createTime"): a missing or invalid timestamp falls back to the current epoch second
fn created_at(
    metadata: Option<&Json>,
    now: i64,
    local: &LocalTimeContext,
) -> Result<i64, VeoOperationError> {
    let fields = match metadata {
        None | Some(Json::Null) => return Ok(now),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VeoOperationError::MetadataShape);
        }
    };
    let Some(Json::Text(text)) = fields.get("createTime") else {
        return Ok(now);
    };
    Ok(decode_vertex_timestamp(text.as_bytes(), local).unwrap_or(now))
}
