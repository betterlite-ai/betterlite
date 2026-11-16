use std::convert::Infallible;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_anthropic::stream_decode::error::ReceiveError;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub(crate) enum HelperFailure {
    #[error("{message}")]
    Unknown { message: String },
    #[error("the transport timed out")]
    Timeout,
    #[error("the fixture has no reply left")]
    Exhausted,
}

pub(crate) enum Verdict {
    Reply {
        status_code: u16,
        headers: HeaderMap,
        lines: Vec<String>,
    },
    Failed(PostFailure<HelperFailure>),
}

pub(crate) struct FixtureTransport {
    verdict: Mutex<Option<Verdict>>,
    pub(crate) posts: Mutex<Vec<StreamPost>>,
    pub(crate) polled: Arc<AtomicBool>,
}

impl FixtureTransport {
    pub(crate) fn new(verdict: Verdict) -> Self {
        Self {
            verdict: Mutex::new(Some(verdict)),
            posts: Mutex::new(Vec::new()),
            polled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn reply(status_code: u16, headers: HeaderMap, lines: &[String]) -> Self {
        Self::new(Verdict::Reply {
            status_code,
            headers,
            lines: lines.to_vec(),
        })
    }

    pub(crate) fn status(status_code: u16, body: &str, headers: HeaderMap) -> Self {
        Self::new(Verdict::Failed(PostFailure::Status(StatusFailure {
            status_code,
            body: body.as_bytes().to_vec(),
            // the client raised on the response, so the headers come from the response itself
            exception_headers: None,
            response_headers: Some(headers),
        })))
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = HelperFailure;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<HelperFailure>, PostFailure<HelperFailure>>> + Send
    {
        async move {
            self.posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(post);
            let taken = self
                .verdict
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let verdict = taken.ok_or(PostFailure::Unknown(HelperFailure::Exhausted))?;
            let (status_code, headers, lines) = match verdict {
                Verdict::Failed(failure) => return Err(failure),
                Verdict::Reply {
                    status_code,
                    headers,
                    lines,
                } => (status_code, headers, lines),
            };
            let flag = Arc::clone(&self.polled);
            let items = lines
                .into_iter()
                .map(|line| Ok(TransportChunk::Text(line)))
                .collect::<Vec<Result<TransportChunk, ReceiveError<HelperFailure>>>>();
            let lines = stream::iter(items)
                .inspect(move |_item| flag.store(true, Ordering::SeqCst))
                .boxed();
            Ok(StreamReply {
                status_code,
                headers,
                lines,
            })
        }
    }
}

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

pub(crate) struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}
