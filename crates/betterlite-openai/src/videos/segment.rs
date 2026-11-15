use betterlite_http::url_path::encode_segment;

use crate::openai_video_identifiers::ids::decode_video_id_with_provider;
use crate::videos::error::VideoError;

// every video path joins api_base without a trailing slash, then the encoded id and its suffix
pub(crate) fn path_url(video_id: &str, api_base: &str, suffix: &str) -> Result<String, VideoError> {
    let original = decode_video_id_with_provider(video_id).video_id;
    let encoded = encode_segment(&original, "video_id")?;
    Ok(format!(
        "{}/{encoded}{suffix}",
        api_base.trim_end_matches('/')
    ))
}
