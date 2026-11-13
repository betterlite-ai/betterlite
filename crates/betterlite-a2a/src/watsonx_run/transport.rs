use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::time::Duration;

use super::run_api::RunBody;

pub const MAX_RESPONSE_BYTES: usize = 0x80_0000;

#[derive(Clone, Copy, Debug)]
pub struct RunLimits {
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

pub struct RunRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, String>,
    pub body: Option<&'request RunBody>,
    // a transport stops reading a body past limits.max_response_bytes and reports it as too large
    pub limits: RunLimits,
}

#[derive(Debug)]
pub enum RunFailure<E> {
    Transport(E),
    // LiteLLM raises its own Timeout for a request that never got headers, outside every TransportError handler
    Timeout,
}

#[derive(Debug)]
pub struct RunResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct StreamResponse<E> {
    pub status: u16,
    pub content_type: String,
    pub body: StreamBody<E>,
}

#[derive(Debug)]
pub enum StreamBody<E> {
    Chunks(Vec<Vec<u8>>),
    ReadFailed(E),
    TooLarge { max: usize },
}

pub trait RunTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn post(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, RunFailure<Self::Error>>> + Send;

    fn get(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<RunResponse, RunFailure<Self::Error>>> + Send;

    fn post_stream(
        &self,
        request: &RunRequest<'_>,
    ) -> impl Future<Output = Result<StreamResponse<Self::Error>, RunFailure<Self::Error>>> + Send;

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}
