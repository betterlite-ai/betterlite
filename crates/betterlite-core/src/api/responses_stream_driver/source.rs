use std::collections::VecDeque;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::timed_out;

use super::errors::{TransportFailure, TransportKind};

// B173's reqwest errors map onto the httpx family LiteLLM's iterator branches on.
#[must_use]
pub fn classify(error: HttpError) -> TransportFailure {
    match timed_out(error) {
        HttpError::Request(source) => {
            TransportFailure::new(TransportKind::ReadError, &source.to_string())
        }
        other => TransportFailure::new(TransportKind::HttpError, &other.to_string()),
    }
}
#[derive(Debug)]
pub enum SourceFailure {
    Transport(TransportFailure),
    // the SSE frame itself could not be read, which LiteLLM never produces
    Decode(String),
}
#[async_trait]
pub trait PayloadSource: Send {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>>;
}
// collecting the body first lets a sync step run without an executor
#[derive(Default)]
pub struct PrefetchedSource {
    items: VecDeque<Result<Vec<u8>, SourceFailure>>,
}
impl PrefetchedSource {
    #[must_use]
    pub fn new(items: Vec<Result<Vec<u8>, SourceFailure>>) -> Self {
        Self {
            items: items.into(),
        }
    }
}
#[async_trait]
impl PayloadSource for PrefetchedSource {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        self.items.pop_front()
    }
}
