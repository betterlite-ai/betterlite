use std::fmt::{self, Debug, Formatter};
use std::pin::pin;
use std::time::Duration;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_redact::secret::Redactor;
use futures_util::{Stream, StreamExt as _};
use http::{HeaderMap, HeaderName, StatusCode};
use tokio::time::timeout;

use super::secrets::request_secret_values;
use super::text::safe_upstream_url;
use super::{
    BODY_CAPTURE_BYTES, CAPTURE_FAILED, CAPTURE_TIMEOUT_SECONDS, CREDENTIALS_UNAVAILABLE,
    content_type, preview,
};

#[derive(Clone, PartialEq, Eq)]
pub enum BodyState {
    NotRead,
    Read(Vec<u8>),
}

// LiteLLM keeps the credentials a preview masks; Debug reports only sizes and names
impl Debug for BodyState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRead => f.write_str("NotRead"),
            Self::Read(raw) => f.debug_struct("Read").field("bytes", &raw.len()).finish(),
        }
    }
}

#[derive(Clone)]
pub struct UpstreamRequest {
    pub method: String,
    pub url: HttpxUrl,
    pub headers: HeaderMap,
    pub body: BodyState,
}

impl Debug for UpstreamRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        f.debug_struct("UpstreamRequest")
            .field("method", &self.method)
            .field("url", &safe_upstream_url(&self.url))
            .field("headers", &header_names)
            .field("body", &self.body)
            .finish()
    }
}

#[derive(Clone)]
pub struct UpstreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: BodyState,
    pub request: Option<UpstreamRequest>,
    pub captured: Option<String>,
}

impl Debug for UpstreamResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl UpstreamResponse {
    fn summary(&self) -> String {
        let names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        format!(
            "UpstreamResponse({}, headers={names:?}, body={:?}, request={:?}, captured={:?})",
            self.status.as_u16(),
            self.body,
            self.request,
            self.captured
        )
    }
}

pub trait UpstreamFailure {
    fn response(&self) -> Option<&UpstreamResponse>;
    fn cause(&self) -> Option<&dyn UpstreamFailure>;
}

enum ReadPrefix {
    Complete(Vec<u8>),
    Failed,
}

pub async fn capture_error_response<Chunks, ChunkError>(
    response: &mut UpstreamResponse,
    chunks: Chunks,
    redactor: &Redactor,
) where
    Chunks: Stream<Item = Result<Vec<u8>, ChunkError>>,
{
    if !response.status.is_client_error() && !response.status.is_server_error() {
        return;
    }
    let limit = BODY_CAPTURE_BYTES.saturating_add(1);
    let read = timeout(
        Duration::from_secs(CAPTURE_TIMEOUT_SECONDS),
        read_prefix(chunks, limit),
    )
    .await;
    let prefix = match read {
        Ok(ReadPrefix::Complete(prefix)) => prefix,
        Ok(ReadPrefix::Failed) | Err(_) => {
            response.body = BodyState::Read(Vec::new());
            response.captured = Some(CAPTURE_FAILED.to_owned());
            return;
        }
    };
    let secrets = response.request.as_ref().and_then(request_secret_values);
    let captured = match secrets {
        None => CREDENTIALS_UNAVAILABLE.to_owned(),
        Some(found) => preview(&prefix, &content_type(&response.headers), &found, redactor),
    };
    response.body = BodyState::Read(prefix);
    response.captured = Some(captured);
}

async fn read_prefix<Chunks, ChunkError>(chunks: Chunks, limit: usize) -> ReadPrefix
where
    Chunks: Stream<Item = Result<Vec<u8>, ChunkError>>,
{
    let mut buffer: Vec<u8> = Vec::new();
    let mut pinned = pin!(chunks);
    while buffer.len() < limit {
        let Some(next_chunk) = pinned.next().await else {
            return ReadPrefix::Complete(buffer);
        };
        let Ok(bytes) = next_chunk else {
            return ReadPrefix::Failed;
        };
        let remaining = limit.saturating_sub(buffer.len());
        buffer.extend(bytes.into_iter().take(remaining));
    }
    ReadPrefix::Complete(buffer)
}
