use std::collections::VecDeque;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use crate::api::responses_stream_driver::source::{PayloadSource, SourceFailure, classify};

// The live response body as B927 reads it: SSE frames parsed off the wire, errors classified as transport failures
pub(super) struct WireSource {
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
}

impl WireSource {
    #[must_use]
    pub(super) fn new(response: StreamResponse) -> Self {
        Self {
            body: response.body_stream().boxed(),
            parser: SseParser::default(),
            pending: VecDeque::new(),
        }
    }
}

#[async_trait]
impl PayloadSource for WireSource {
    async fn next_payload(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        loop {
            if let Some(payload) = self.pending.pop_front() {
                return Some(Ok(payload));
            }
            match self.body.next().await {
                Some(Ok(chunk)) => match self.parser.push(&chunk) {
                    Ok(payloads) => self.pending.extend(payloads),
                    Err(error) => return Some(Err(SourceFailure::Decode(error.to_string()))),
                },
                Some(Err(error)) => return Some(Err(SourceFailure::Transport(classify(error)))),
                None => {
                    self.pending.extend(self.parser.finish());
                    if let Some(payload) = self.pending.pop_front() {
                        return Some(Ok(payload));
                    }
                    return None;
                }
            }
        }
    }
}
