use std::future::Future;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::call_helpers::headers::{AdditionalHeaders, set_provider_response_headers};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::files::stream::flatten_headers;
use crate::speech::error::SpeechError;

// One buffered upstream answer for a call the SDK would raise on before handing back a body.
pub struct SpeechReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

// B173's pooled client in production, a recorded reply in tests and in the oracle helper
pub trait SpeechTransport: Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<SpeechReply, TransportError>> + Send;
}

impl SpeechTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<SpeechReply, TransportError> {
        let response = ProxiedClient::send_following(self, request).await?;
        Ok(SpeechReply {
            status: response.response.status().as_u16(),
            headers: response.response.headers().clone(),
            body: response.body_stream(),
        })
    }
}

// LiteLLM's HttpxBinaryResponseContent: the provider's bytes, headers and the B206 header projection, never JSON or text.
pub struct SpeechResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: IndexMap<String, String>,
    pub additional_headers: AdditionalHeaders,
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

impl From<SpeechReply> for SpeechResponse {
    fn from(reply: SpeechReply) -> Self {
        let headers = flatten_headers(&reply.headers);
        let projection = set_provider_response_headers(&AdditionalHeaders::default(), &headers);
        let content_type = headers.get("content-type").cloned();
        Self {
            status: reply.status,
            content_type,
            headers,
            additional_headers: projection.additional_headers,
            body: reply.body,
        }
    }
}

impl SpeechResponse {
    // The owned upstream body, read under the caller's limit; dropping the response cancels an unread body.
    pub async fn content(mut self, limit: usize) -> Result<Vec<u8>, SpeechError> {
        let mut collected = Vec::new();
        while let Some(chunk) = self.body.next().await {
            let chunk = chunk.map_err(|source| SpeechError::Body { source })?;
            let Some(total) = collected.len().checked_add(chunk.len()) else {
                return Err(SpeechError::TooLarge);
            };
            if total > limit {
                return Err(SpeechError::TooLarge);
            }
            collected.extend_from_slice(&chunk);
        }
        Ok(collected)
    }
}
