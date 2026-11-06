use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use super::analysis::AnalyzePayload;
use super::transient::TransportError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestTimeout {
    pub total: Duration,
    pub connect: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub body: Vec<u8>,
}

pub trait AsyncPostHandler: Sync {
    fn post<'request>(
        &'request self,
        url: &str,
        headers: &HeaderMap,
        json: &AnalyzePayload,
        timeout: RequestTimeout,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, TransportError>> + Send + 'request>>;
}
