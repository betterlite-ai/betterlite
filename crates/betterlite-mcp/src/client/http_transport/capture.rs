use std::fmt;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, Method};
use url::Url;

use crate::client::cleanup::recording::ResponseBody as _;

use super::error::McpHttpError;
use super::send::HttpBody;

#[derive(Clone)]
pub struct Exchange {
    pub method: Method,
    pub url: Url,
    pub request_headers: HeaderMap,
    pub request_body: Bytes,
    pub status: u16,
    pub response_headers: HeaderMap,
}

// The exchange carries the credentials the preview masks, so Debug reports names and sizes only
impl fmt::Debug for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let request_names: Vec<&str> = self
            .request_headers
            .keys()
            .map(HeaderName::as_str)
            .collect();
        let response_names: Vec<&str> = self
            .response_headers
            .keys()
            .map(HeaderName::as_str)
            .collect();
        f.debug_struct("Exchange")
            .field("method", &self.method)
            .field("url", &self.url.origin().ascii_serialization())
            .field("request_headers", &request_names)
            .field("request_body_bytes", &self.request_body.len())
            .field("status", &self.status)
            .field("response_headers", &response_names)
            .finish()
    }
}

pub struct CapturedBody {
    pub retained: Vec<u8>,
    pub preview: String,
}

pub struct CaptureRecord {
    pub exchange: Exchange,
    pub preview: String,
}

impl fmt::Debug for CaptureRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureRecord")
            .field("exchange", &self.exchange)
            .field("preview_chars", &self.preview.chars().count())
            .finish()
    }
}

pub trait ErrorCapture: Send + Sync {
    fn capture<'body>(
        &'body self,
        exchange: Exchange,
        body: BodyChunks<'body>,
    ) -> Pin<Box<dyn Future<Output = CapturedBody> + Send + 'body>>;
}

pub struct BodyChunks<'body> {
    pub(super) body: &'body mut HttpBody,
    pub(super) limit: usize,
    pub(super) remaining: usize,
}

impl BodyChunks<'_> {
    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, McpHttpError>> {
        match self.body.next_chunk().await? {
            Ok(bytes) if bytes.len() <= self.remaining => {
                self.remaining = self.remaining.saturating_sub(bytes.len());
                Some(Ok(bytes))
            }
            Ok(_) => Some(Err(McpHttpError::BodyTooLarge { limit: self.limit })),
            Err(error) => Some(Err(error)),
        }
    }
}
