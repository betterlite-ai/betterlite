use std::collections::VecDeque;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{StreamResponse, timed_out};
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::errors::{TransportFailure, TransportKind};

/// The provider body in arrival order; a read failure ends it where it happened.
pub type BodyStream = BoxStream<'static, Result<Vec<u8>, TransportFailure>>;

// B173's reqwest errors map onto the httpx family LiteLLM's iterator branches on.
#[must_use]
pub fn body_stream(response: StreamResponse) -> BodyStream {
    response
        .body_stream()
        .map(|chunk| chunk.map_err(classify))
        .boxed()
}

pub(super) fn classify(error: HttpError) -> TransportFailure {
    match timed_out(error) {
        HttpError::Request(source) => {
            TransportFailure::new(TransportKind::ReadError, source.to_string())
        }
        other => TransportFailure::new(TransportKind::HttpError, other.to_string()),
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

/// Payloads already collected from the body, so a sync step never waits on I/O.
#[derive(Default)]
pub struct PrefetchedSource {
    items: VecDeque<Result<Vec<u8>, SourceFailure>>,
}

impl PrefetchedSource {
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = Result<Vec<u8>, SourceFailure>>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }
}

#[async_trait]
impl PayloadSource for PrefetchedSource {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        self.items.pop_front()
    }
}

/// B173 bytes framed by B005's SSE parser one event at a time.
pub struct StreamingSource {
    body: BodyStream,
    parser: SseParser,
    ready: VecDeque<Vec<u8>>,
    ended: bool,
}

impl StreamingSource {
    #[must_use]
    pub fn new(body: BodyStream) -> Self {
        Self {
            body,
            parser: SseParser::default(),
            ready: VecDeque::new(),
            ended: false,
        }
    }
}

#[async_trait]
impl PayloadSource for StreamingSource {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        loop {
            if let Some(payload) = self.ready.pop_front() {
                return Some(Ok(payload));
            }
            if self.ended {
                return None;
            }
            match self.body.next().await {
                Some(Ok(bytes)) => match self.parser.push(&bytes) {
                    Ok(payloads) => self.ready.extend(payloads),
                    Err(error) => {
                        self.ended = true;
                        return Some(Err(SourceFailure::Decode(error.to_string())));
                    }
                },
                Some(Err(failure)) => {
                    self.ended = true;
                    return Some(Err(SourceFailure::Transport(failure)));
                }
                None => {
                    self.ended = true;
                    self.ready.extend(self.parser.finish());
                }
            }
        }
    }
}
