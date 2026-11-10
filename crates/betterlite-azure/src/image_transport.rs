pub mod body;
pub mod decode;
pub mod error;
pub mod json;
pub mod request;

mod tests;

use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use tokio::time::{Instant, sleep};

const SECRET_HEADERS: [&str; 3] = ["authorization", "api-key", "x-api-key"];

#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl ImageRequest {
    #[must_use]
    pub fn new(method: Method, url: String, mut headers: HeaderMap, body: Vec<u8>) -> Self {
        for name in SECRET_HEADERS {
            if let Some(value) = headers.get_mut(name) {
                value.set_sensitive(true);
            }
        }
        Self {
            method,
            url,
            headers,
            body,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub method: Method,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct ImageResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub request: RequestMetadata,
}

pub trait ImageTransport: Send + Sync {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send;
}

pub trait ImageClock: Send + Sync {
    fn now(&self) -> Duration;

    fn pause(&self, seconds: i64) -> impl Future<Output = ()> + Send;
}

pub struct SystemClock {
    pub origin: Instant,
}

impl ImageClock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    async fn pause(&self, seconds: i64) {
        sleep(Duration::from_secs(u64::try_from(seconds).unwrap_or(0))).await;
    }
}
