use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::openai_video_identifiers::ids::decode_video_id_with_provider;
use crate::videos::error::VideoError;

// urllib.parse.quote(safe="") keeps exactly the RFC 3986 unreserved characters unescaped
const QUOTED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[must_use]
pub fn original_video_id(encoded_video_id: &str) -> String {
    decode_video_id_with_provider(encoded_video_id).video_id
}

#[must_use]
pub fn encode_quoted(value: &str) -> String {
    utf8_percent_encode(value, QUOTED).to_string()
}

pub fn encoded_video_path(video_id: &str) -> Result<String, VideoError> {
    let original = original_video_id(video_id);
    if original.is_empty() {
        return Err(VideoError::MissingVideoId);
    }
    if original == "." || original == ".." {
        return Err(VideoError::DotSegment);
    }
    Ok(encode_quoted(&original))
}

// every video path joins api_base without a trailing slash, then the encoded id and its suffix
pub(crate) fn path_url(video_id: &str, api_base: &str, suffix: &str) -> Result<String, VideoError> {
    let encoded = encoded_video_path(video_id)?;
    Ok(format!(
        "{}/{encoded}{suffix}",
        api_base.trim_end_matches('/')
    ))
}
