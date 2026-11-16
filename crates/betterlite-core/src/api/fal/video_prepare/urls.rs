use betterlite_http::url_path::encode_segment;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;

use super::error::FalVideoError;
use super::paths::queue_request_base_path;

pub struct VideoTarget {
    pub request_id: String,
    pub model_id: String,
}

// one job answers both endpoints, so the request path is built once for the two urls
pub struct VideoJobUrls {
    pub status: String,
    pub content: String,
}

pub fn decode_video_id(video_id: &str) -> Result<VideoTarget, FalVideoError> {
    let decoded = decode_video_id_with_provider(video_id);
    let model_id = decoded
        .model_id
        .filter(|model| !model.is_empty())
        .ok_or(FalVideoError::VideoIdWithoutModel)?;
    Ok(VideoTarget {
        request_id: decoded.video_id,
        model_id,
    })
}

pub fn job_urls(video_id: &str, api_base: &str) -> Result<VideoJobUrls, FalVideoError> {
    let target = decode_video_id(video_id)?;
    let encoded = encode_segment(&target.request_id, "video_id")?;
    let request_url = format!(
        "{}/{}/requests/{encoded}",
        api_base.trim_end_matches('/'),
        queue_request_base_path(&target.model_id)
    );
    Ok(VideoJobUrls {
        status: format!("{request_url}/status"),
        content: request_url,
    })
}
