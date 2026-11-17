use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::HeaderMap;
use url::Url;

use super::decode::decode_studio_veo_operation;
use super::error::StudioVeoOperationError;
use super::records::status_request;

// B173's pooled client in production and a scripted reply in tests
pub trait DownloadTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl DownloadTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx clients follow redirects, so each hop is checked again; the artifact stays under the body cap
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

// the caller-side inputs the content request reads: the encoded video id, the API base, the supplied headers and the deadline
pub struct ContentLookup<'call> {
    pub video_id: &'call str,
    pub api_base: &'call str,
    pub headers: &'call HeaderMap,
    pub deadline: Duration,
}

// the download the content request resolved: the provider artifact URI, the same headers and the deadline
pub struct DownloadRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub deadline: Duration,
}

// transform_video_content_request: poll the operation, then hand the first generated sample's URI to the download; raise_for_status fails on every status outside 2xx
pub async fn content_request<Transport>(
    lookup: &ContentLookup<'_>,
    transport: &Transport,
) -> Result<DownloadRequest, StudioVeoOperationError>
where
    Transport: DownloadTransport,
{
    let request = status_outbound(lookup)?;
    let url = request.url.to_string();
    let reply = match transport.send(request).await {
        Ok(reply) => reply,
        Err(source) => return Err(StudioVeoOperationError::Transport { url, source }),
    };
    if !(200..300).contains(&reply.status) {
        return Err(StudioVeoOperationError::Provider {
            url,
            response: Box::new(reply),
        });
    }
    let target = download_target(&reply.body)?;
    let target_url = match Url::parse(&target) {
        Ok(parsed) => parsed,
        Err(source) => {
            return Err(StudioVeoOperationError::Url {
                url: target,
                source,
            });
        }
    };
    Ok(DownloadRequest {
        url: target_url,
        headers: lookup.headers.clone(),
        deadline: lookup.deadline,
    })
}

// the status body of a completed operation names the artifact to download; an empty sample list fails instead of moving on
pub fn download_target(body: &[u8]) -> Result<String, StudioVeoOperationError> {
    let operation = decode_studio_veo_operation(body)?;
    if !operation.done {
        return Err(StudioVeoOperationError::NotComplete);
    }
    let Some(response) = operation.response else {
        return Err(StudioVeoOperationError::NoResponse);
    };
    let Some(sample) = response
        .generate_video_response
        .generated_samples
        .into_iter()
        .next()
    else {
        return Err(StudioVeoOperationError::NoSamples);
    };
    Ok(sample.video.uri)
}

// transform_video_content_response: the provider's artifact bytes come back unchanged, whatever status carried them
pub async fn download_content<Transport>(
    request: DownloadRequest,
    transport: &Transport,
) -> Result<Vec<u8>, StudioVeoOperationError>
where
    Transport: DownloadTransport,
{
    let url = request.url.to_string();
    let reply = match transport
        .send(OutboundRequest {
            method: Method::GET,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Empty,
            deadline: request.deadline,
        })
        .await
    {
        Ok(reply) => reply,
        Err(source) => return Err(StudioVeoOperationError::Transport { url, source }),
    };
    Ok(reply.body)
}

// the poll answers the empty parameter map that the status transform returns, so the GET carries no query
fn status_outbound(lookup: &ContentLookup<'_>) -> Result<OutboundRequest, StudioVeoOperationError> {
    let poll = status_request(lookup.video_id, lookup.api_base);
    let url = match Url::parse(&poll.url) {
        Ok(url) => url,
        Err(source) => {
            return Err(StudioVeoOperationError::Url {
                url: poll.url,
                source,
            });
        }
    };
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: lookup.headers.clone(),
        body: RequestBody::Empty,
        deadline: lookup.deadline,
    })
}

mod tests;
