use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_http::error::HttpError;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use futures_util::Stream;

pub mod error;
pub mod events;
pub mod project;

use error::SagemakerStreamError;
use events::{EventItem, EventState};

// the sync iterator lets a failure end the stream, the async one logs it and reads the next event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPolicy {
    Propagate,
    Skip,
}

#[derive(Debug)]
pub enum SagemakerChunk {
    Chat(Chunk),
    Token(GenericChunk),
}

pub struct SagemakerStreamDecoder {
    model: String,
    is_messages_api: bool,
    policy: ErrorPolicy,
}

impl SagemakerStreamDecoder {
    #[must_use]
    pub fn new(model: &str, is_messages_api: bool, policy: ErrorPolicy) -> Self {
        Self {
            model: model.to_owned(),
            is_messages_api,
            policy,
        }
    }

    #[must_use]
    pub fn events<Chunks>(&self, chunks: Chunks) -> SagemakerEvents<Chunks> {
        SagemakerEvents {
            state: EventState::new(&self.model, self.is_messages_api, self.policy),
            chunks,
            pending: VecDeque::new(),
            done: false,
        }
    }
}

// The blocking face reads chunks through Iterator and the async one through Stream, as the sync and async decoders do.
pub struct SagemakerEvents<Chunks> {
    state: EventState,
    chunks: Chunks,
    pending: VecDeque<EventItem>,
    done: bool,
}

impl<Chunks> Iterator for SagemakerEvents<Chunks>
where
    Chunks: Iterator<Item = Result<Vec<u8>, HttpError>>,
{
    type Item = EventItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.done {
                return None;
            }
            match self.chunks.next() {
                Some(Ok(bytes)) => {
                    if !self.state.absorb(&bytes, &mut self.pending) {
                        self.done = true;
                    }
                }
                Some(Err(source)) => {
                    self.pending
                        .push_back(Err(SagemakerStreamError::Body(source)));
                    self.done = true;
                }
                None => {
                    self.done = true;
                    self.state.finish(&mut self.pending);
                }
            }
        }
    }
}

impl<Chunks> Stream for SagemakerEvents<Chunks>
where
    Chunks: Stream<Item = Result<Vec<u8>, HttpError>> + Unpin,
{
    type Item = EventItem;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.pending.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.chunks).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(bytes))) => {
                    if !this.state.absorb(&bytes, &mut this.pending) {
                        this.done = true;
                    }
                }
                Poll::Ready(Some(Err(source))) => {
                    this.pending
                        .push_back(Err(SagemakerStreamError::Body(source)));
                    this.done = true;
                }
                Poll::Ready(None) => {
                    this.done = true;
                    this.state.finish(&mut this.pending);
                }
            }
        }
    }
}

mod tests;
