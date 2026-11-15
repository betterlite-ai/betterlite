use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::reply::{VideoObject, decode_video_reply, expect_video, wrap_video_id};
use crate::videos::segment::path_url;

// transform_video_status_retrieve_request: GET <api_base>/<encoded original id>
pub fn status_request(
    video_id: &str,
    api_base: &str,
) -> Result<(String, VideoOptions), VideoError> {
    Ok((path_url(video_id, api_base, "")?, VideoOptions::new()))
}

// transform_video_status_retrieve_response: wraps a truthy id with model None and keeps usage
pub fn status_response(body: &[u8], provider: Option<&str>) -> Result<VideoObject, VideoError> {
    let mut video = expect_video(decode_video_reply(body)?)?;
    wrap_video_id(&mut video, provider, None);
    Ok(video)
}
