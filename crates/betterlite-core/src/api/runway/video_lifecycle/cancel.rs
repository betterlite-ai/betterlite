use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};

use super::error::VideoLifecycleError;
use crate::api::runway::tasks::record::decode_runway_task;
use crate::api::runway::video_create::reply::parse_timestamp;

const CANCELLED: &str = "cancelled";

// transform_video_delete_response: the reply is always cancelled, keeps the provider id raw, and adds no usage
pub fn cancel_response(body: &[u8]) -> Result<VideoObject, VideoLifecycleError> {
    let task = decode_runway_task(body)?;
    Ok(VideoObject {
        id: task.id.unwrap_or_default(),
        object: VideoObjectType::Video,
        status: CANCELLED.to_owned(),
        created_at: Some(parse_timestamp(task.created_at.value())),
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
