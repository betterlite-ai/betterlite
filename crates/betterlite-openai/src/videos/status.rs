use crate::videos::error::VideoError;
use crate::videos::reply::{VideoObject, decode_video, wrap_video_id};

// transform_video_status_retrieve_response: wraps a truthy id with model None and keeps usage
pub fn status_response(body: &[u8], provider: Option<&str>) -> Result<VideoObject, VideoError> {
    let mut video = decode_video(body)?;
    wrap_video_id(&mut video, provider, None);
    Ok(video)
}
