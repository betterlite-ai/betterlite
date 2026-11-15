use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::reply::{VideoObject, decode_video_reply, expect_video};
use crate::videos::segment::path_url;

// transform_video_delete_request: DELETE <api_base>/<encoded original id>
pub fn delete_request(
    video_id: &str,
    api_base: &str,
) -> Result<(String, VideoOptions), VideoError> {
    Ok((path_url(video_id, api_base, "")?, VideoOptions::new()))
}

// transform_video_delete_response: validates VideoObject and leaves its id and usage alone
pub fn delete_response(body: &[u8]) -> Result<VideoObject, VideoError> {
    expect_video(decode_video_reply(body)?)
}
