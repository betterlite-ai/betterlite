pub mod artifact;
pub mod cancel;
pub mod content;
pub mod error;
pub mod retrieve;
pub mod unsupported;

mod tests;

use betterlite_http::url_path::encode_segment;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;

use self::error::VideoLifecycleError;
use super::video_create::{DEFAULT_API_BASE, VideoParams};

// the URL and empty request data every lifecycle call hands the common executor
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleRequest {
    pub url: String,
    pub data: VideoParams,
}

// the three task paths LiteLLM transforms; content carries the variant the source accepts and never reads
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleOperation<'a> {
    Content { variant: Option<&'a str> },
    Status,
    Cancel,
}

// extract_original_video_id then encode_url_path_segment: a wrapped id is decoded, and the provider id is escaped
pub fn lifecycle_request(
    operation: LifecycleOperation<'_>,
    video_id: &str,
    api_base: Option<&str>,
) -> Result<LifecycleRequest, VideoLifecycleError> {
    let suffix = match operation {
        LifecycleOperation::Cancel => "/cancel",
        LifecycleOperation::Content { .. } | LifecycleOperation::Status => "",
    };
    let original = decode_video_id_with_provider(video_id).video_id;
    let segment = encode_segment(&original, "video_id")?;
    let base = api_base.unwrap_or(DEFAULT_API_BASE).trim_end_matches('/');
    Ok(LifecycleRequest {
        url: format!("{base}/tasks/{segment}{suffix}"),
        data: VideoParams::new(),
    })
}
