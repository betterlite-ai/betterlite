use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
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

pub(crate) type Verdict = Result<StreamReply<HelperFailure>, PostFailure<HelperFailure>>;

pub(crate) struct FixtureTransport {
    verdict: Mutex<Option<Verdict>>,
    pub(crate) posts: Mutex<Vec<StreamPost>>,
    pub(crate) polled: Arc<AtomicBool>,
}

impl FixtureTransport {
    pub(crate) const fn new(verdict: Verdict, polled: Arc<AtomicBool>) -> Self {
        Self {
            verdict: Mutex::new(Some(verdict)),
            posts: Mutex::new(Vec::new()),
            polled,
        }
    }

    pub(crate) fn reply(status_code: u16, headers: HeaderMap, lines: &[String]) -> Self {
        let polled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&polled);
        let items = lines
            .iter()
            .map(|line| Ok(TransportChunk::Text(line.clone())))
            .collect::<Vec<Result<TransportChunk, ReceiveError<HelperFailure>>>>();
        let chunks = stream::iter(items)
            .inspect(move |_item| flag.store(true, Ordering::SeqCst))
            .boxed();
        let reply = StreamReply {
            status_code,
            headers,
            lines: chunks,
        };
        Self::new(Ok(reply), polled)
    }

    pub(crate) fn status(status_code: u16, body: &str, headers: HeaderMap) -> Self {
        let failure = PostFailure::Status(Box::new(StatusFailure {
            status_code,
            body: body.as_bytes().to_vec(),
            // the client raised on the response, so the headers come from the response itself
            exception_headers: None,
            response_headers: Some(headers),
        }));
        Self::new(Err(failure), Arc::default())
    }

    fn respond(
        &self,
        post: StreamPost,
    ) -> Result<StreamReply<HelperFailure>, PostFailure<HelperFailure>> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(post);
        let taken = self
            .verdict
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        taken.unwrap_or(Err(PostFailure::Unknown(HelperFailure::Exhausted)))
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = HelperFailure;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<HelperFailure>, PostFailure<HelperFailure>>> + Send
    {
        ready(self.respond(post))
    }
}

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

pub(crate) struct SilentLogger;

impl StreamLogger for SilentLogger {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

pub(crate) struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}
