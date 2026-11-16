use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use serde::Serialize;

use super::error::VideoLifecycleError;
use crate::api::runway::tasks::record::{TaskValue, decode_runway_task};
use crate::api::runway::video_create::reply::{map_status, parse_timestamp};
use crate::api::runway::video_create::usage::progress_percent;

const DEFAULT_FAILURE_CODE: &str = "unknown";
const DEFAULT_FAILURE_MESSAGE: &str = "Video generation failed";

// VideoObject declares error as its own serde map, so the two provider fields reach it through the model's JSON
#[derive(Serialize)]
struct FailureText {
    code: Json,
    message: Json,
}

// transform_video_status_retrieve_response: LiteLLM also fills output_url, which its VideoObject cannot declare
pub fn retrieve_response(
    body: &[u8],
    provider: Option<&str>,
) -> Result<VideoObject, VideoLifecycleError> {
    let task = decode_runway_task(body)?;
    // the error object exists once the task carries either failure field, and each missing one takes its default
    let failure = if matches!(task.failure_code, TaskValue::Missing)
        && matches!(task.failure, TaskValue::Missing)
    {
        None
    } else {
        serde_json::to_value(FailureText {
            code: task.failure_code.or_default(DEFAULT_FAILURE_CODE),
            message: task.failure.or_default(DEFAULT_FAILURE_MESSAGE),
        })
        .ok()
        .and_then(|value| value.as_object().cloned())
    };
    let mut video = VideoObject {
        id: task.id.unwrap_or_default(),
        object: VideoObjectType::Video,
        status: map_status(task.status.as_deref()).to_owned(),
        created_at: Some(parse_timestamp(task.created_at.value())),
        completed_at: task
            .completed_at
            .value()
            .map(|timestamp| parse_timestamp(Some(timestamp))),
        expires_at: None,
        error: failure,
        progress: task.progress.map(progress_percent),
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: None,
        usage: None,
    };
    if let Some(name) = provider {
        video.id = encode_video_id_with_provider(&video.id, name, None);
    }
    Ok(video)
}
