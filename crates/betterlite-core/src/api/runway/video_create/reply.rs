use betterlite_models::decisions::Json;
use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType, duration_usage};
use serde_json::json;

use super::VideoParams;
use super::error::RunwayVideoError;
use super::usage::{estimated_cost_usd, ratio_to_resolution};
use crate::api::runway::tasks::record::{TaskValue, decode_runway_task};

const DEFAULT_STATUS: &str = "pending";
const DEFAULT_FAILURE_CODE: &str = "unknown";
const DEFAULT_FAILURE_MESSAGE: &str = "Video generation failed";

// _map_runway_status: the provider's name in any case, and every name it does not list is queued
#[must_use]
pub fn map_status(status: Option<&str>) -> &'static str {
    match status.unwrap_or(DEFAULT_STATUS).to_uppercase().as_str() {
        "SUCCEEDED" => "completed",
        "RUNNING" => "in_progress",
        "FAILED" | "CANCELLED" => "failed",
        _ => "queued",
    }
}

// _parse_runway_timestamp: int(dt.timestamp()) of ISO text, and zero for any other value the reply carries
#[must_use]
pub fn parse_timestamp(timestamp: Option<&Json>) -> i64 {
    let Some(Json::Text(text)) = timestamp else {
        return 0;
    };
    let Some(parsed) = decode_billed_at(text) else {
        return 0;
    };
    let seconds = parsed.timestamp();
    // int() truncates toward zero while chrono's timestamp floors a negative instant
    if seconds < 0 && parsed.timestamp_subsec_nanos() > 0 {
        seconds.saturating_add(1)
    } else {
        seconds
    }
}

// str(request_data["duration"]): a scalar keeps its spelling, and the Python text of any other value is not ported
fn duration_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Bool(_) | Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// transform_video_create_response: the task record becomes a VideoObject, and pydantic drops the output URL it cannot declare
pub fn create_response(
    model: &str,
    body: &[u8],
    provider: Option<&str>,
    request_data: &VideoParams,
) -> Result<VideoObject, RunwayVideoError> {
    let task = decode_runway_task(body)?;
    let mut video = VideoObject {
        id: task.id.clone().unwrap_or_default(),
        object: VideoObjectType::Video,
        status: map_status(task.status.as_deref()).to_owned(),
        created_at: Some(parse_timestamp(task.created_at.value())),
        completed_at: task
            .completed_at
            .value()
            .map(|timestamp| parse_timestamp(Some(timestamp))),
        expires_at: None,
        error: None,
        progress: None,
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: None,
        usage: None,
    };
    if let Some(Json::Text(requested_model)) = request_data.get("model") {
        video.model = Some(requested_model.clone());
    }
    if let Some(Json::Text(ratio)) = request_data.get("ratio")
        && ratio.contains(':')
    {
        video.size = Some(ratio.replace(':', "x"));
    }
    if let Some(duration) = request_data.get("duration") {
        video.seconds = duration_text(duration).map(|text| json!(text));
    }
    if !matches!(task.failure_code, TaskValue::Missing)
        || !matches!(task.failure, TaskValue::Missing)
    {
        let error = json!({
            "code": task.failure_code.or_default(DEFAULT_FAILURE_CODE),
            "message": task.failure.or_default(DEFAULT_FAILURE_MESSAGE),
        });
        video.error = error.as_object().cloned();
    }
    if let Some(name) = provider {
        video.id = encode_video_id_with_provider(&video.id, name, Some(model));
    }
    let mut usage = duration_usage(&video);
    if let Some(resolution) = ratio_to_resolution(request_data.get("ratio")) {
        usage.insert("video_resolution".to_owned(), json!(resolution));
    }
    if let Some(usd) = estimated_cost_usd(task.estimated_credits()) {
        usage.insert("provider_reported_cost_usd".to_owned(), json!(usd));
    }
    video.usage = Some(usage);
    Ok(video)
}
