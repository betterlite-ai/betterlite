use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::segment::{encode_quoted, path_url};

// transform_video_content_request: the variant is a query value, percent-encoded like the path
pub fn content_request(
    video_id: &str,
    api_base: &str,
    variant: Option<&str>,
) -> Result<(String, VideoOptions), VideoError> {
    let mut url = path_url(video_id, api_base, "/content")?;
    if let Some(variant) = variant {
        url.push_str("?variant=");
        url.push_str(&encode_quoted(variant));
    }
    Ok((url, VideoOptions::new()))
}

// transform_video_content_response: the reply bytes are handed back untouched
#[must_use]
pub fn content_response(body: &[u8]) -> Vec<u8> {
    body.to_vec()
}
