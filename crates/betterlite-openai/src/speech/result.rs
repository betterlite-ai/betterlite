use betterlite_http::error::HttpError;
use betterlite_models::call_helpers::headers::{AdditionalHeaders, set_provider_response_headers};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use indexmap::IndexMap;

use crate::files::stream::flatten_headers;
use crate::files::transport::StreamReply;
use crate::speech::error::SpeechError;

// LiteLLM's HttpxBinaryResponseContent: the provider's bytes, headers and the B206 header projection, never JSON or text.
pub struct SpeechResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: IndexMap<String, String>,
    pub additional_headers: AdditionalHeaders,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

impl From<StreamReply> for SpeechResponse {
    fn from(reply: StreamReply) -> Self {
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
            let bytes = chunk.map_err(|source| SpeechError::Body { source })?;
            let Some(total) = collected.len().checked_add(bytes.len()) else {
                return Err(SpeechError::TooLarge);
            };
            if total > limit {
                return Err(SpeechError::TooLarge);
            }
            collected.extend_from_slice(&bytes);
        }
        Ok(collected)
    }
}
