use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::segment::path_url;

// Python's quote(value, safe="") keeps only the RFC 3986 unreserved characters; an empty variant is valid
const VARIANT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

// transform_video_content_request: the variant is a query value, percent-encoded like the path
pub fn content_request(
    video_id: &str,
    api_base: &str,
    variant: Option<&str>,
) -> Result<(String, VideoOptions), VideoError> {
    let mut url = path_url(video_id, api_base, "/content")?;
    if let Some(value) = variant {
        url.push_str("?variant=");
        url.push_str(&utf8_percent_encode(value, VARIANT).to_string());
    }
    Ok((url, VideoOptions::new()))
}
