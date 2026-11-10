use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use http::HeaderMap;
use url::Url;

#[derive(Debug)]
pub struct HttpRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Arc<[u8]>,
    pub timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
}

pub trait Transport: Send + Sync + Debug {
    type Error: Error + Send + Sync + 'static;

    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, Self::Error>> + Send;

    // LiteLLM retries only httpx.TransportError and timeouts, so a transport names the errors that end the attempt
    fn retryable(_error: &Self::Error) -> bool {
        true
    }
}
