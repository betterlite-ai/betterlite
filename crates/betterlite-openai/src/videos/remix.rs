use serde_json::Value;

use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::reply::{
    VideoObject, decode_video_reply, duration_usage, expect_video, wrap_video_id,
};
use crate::videos::segment::path_url;

// transform_video_remix_request: the body starts with prompt, and extra_body can overwrite it
pub fn remix_request(
    video_id: &str,
    prompt: &str,
    api_base: &str,
    extra_body: Option<&VideoOptions>,
) -> Result<(String, VideoOptions), VideoError> {
    let url = path_url(video_id, api_base, "/remix")?;
    let mut data = VideoOptions::new();
    data.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    if let Some(extra_body) = extra_body {
        for (name, value) in extra_body {
            data.insert(name.clone(), value.clone());
        }
    }
    Ok((url, data))
}

// transform_video_remix_response: the wrapped id carries no model, and usage is rebuilt
pub fn remix_response(body: &[u8], provider: Option<&str>) -> Result<VideoObject, VideoError> {
    let mut video = expect_video(decode_video_reply(body)?)?;
    wrap_video_id(&mut video, provider, None);
    video.usage = Some(duration_usage(&video));
    Ok(video)
}
