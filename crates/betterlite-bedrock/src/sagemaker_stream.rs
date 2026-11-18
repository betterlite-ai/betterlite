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
    pub model: String,
    pub is_messages_api: bool,
    pub policy: ErrorPolicy,
}

// The blocking face reads chunks through Iterator and the async one through Stream, as the sync and async decoders do.
pub struct SagemakerEvents<Chunks> {
    pub state: EventState,
    pub chunks: Chunks,
    pub pending: VecDeque<EventItem>,
    pub done: bool,
}

impl<Chunks> SagemakerEvents<Chunks> {
    // a failed absorb ends the stream: the frame or projection error is already pending
    fn accept(&mut self, bytes: &[u8]) {
        if !self.state.absorb(bytes, &mut self.pending) {
            self.done = true;
        }
    }

    fn source_failed(&mut self, source: HttpError) {
        self.pending
            .push_back(Err(SagemakerStreamError::Body(source)));
        self.done = true;
    }

    fn exhausted(&mut self) -> Option<EventItem> {
        self.done = true;
        self.state.finish(&mut self.pending);
        self.pending.pop_front()
    }
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
                Some(Ok(bytes)) => self.accept(&bytes),
                Some(Err(source)) => self.source_failed(source),
                None => return self.exhausted(),
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
                Poll::Ready(Some(Ok(bytes))) => this.accept(&bytes),
                Poll::Ready(Some(Err(source))) => this.source_failed(source),
                Poll::Ready(None) => return Poll::Ready(this.exhausted()),
            }
        }
    }
}
mod tests;
