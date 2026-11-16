use http::header::HeaderMap;
use url::Url;

use crate::api::fal::video_prepare::decode::{FalVideoReply, decode_fal_video_reply};
use crate::api::fal::video_prepare::error::get_error_class;

use super::UpstreamReply;
use super::error::VideoResultsError;
use super::text::{error_text, result_error};
use super::transport::{ClientKind, VideoClients, VideoTransport};

pub async fn content_response<T: VideoTransport + Sync>(
    clients: &VideoClients<'_, T>,
    kind: ClientKind,
    reply: &UpstreamReply,
) -> Result<Vec<u8>, VideoResultsError> {
    if let Some(message) = result_error(reply.status, &reply.body) {
        return Err(VideoResultsError::Upstream(get_error_class(
            &message,
            reply.status,
            &reply.headers,
        )));
    }
    let decoded = decode_fal_video_reply(reply.status, &reply.headers, &reply.body)?;
    let url = extract_video_url(&decoded)?;
    let artifact = Url::parse(&url).map_err(VideoResultsError::ArtifactUrl)?;
    // LiteLLM fetches the provider-produced artifact from a freshly built client and forwards none of the queue credentials
    let response = clients
        .for_kind(kind)
        .get(artifact, HeaderMap::new())
        .await
        .map_err(VideoResultsError::ArtifactTransport)?;
    if !(200..=299).contains(&response.status) {
        return Err(VideoResultsError::ArtifactStatus {
            status: response.status,
        });
    }
    Ok(response.body)
}

#[must_use]
pub fn extract_video_url(reply: &FalVideoReply) -> Result<String, VideoResultsError> {
    if let Some(url) = reply.video.as_ref().and_then(|video| video.url.as_deref()) {
        return Ok(url.to_owned());
    }
    match error_text(reply).filter(|text| !text.is_empty()) {
        Some(detail) => Err(VideoResultsError::MissingVideoUrlDetail { detail }),
        None => Err(VideoResultsError::MissingVideoUrl),
    }
}
