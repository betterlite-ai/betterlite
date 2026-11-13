use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use bytes::{Bytes, BytesMut};
use futures_util::Stream;

use crate::event_frames::AwsEvent;
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;
use crate::invoke_events::InvokeChunk;
use crate::invoke_events::InvokeStreamDecoder;
use crate::invoke_events::error::InvokeStreamError;

// The bytes B599 hands back for one framed Bedrock event; every provider projection reads its JSON.
pub fn decode_bedrock_event_json(payload: &[u8]) -> Result<Json, InvokeStreamError> {
    match serde_json::from_slice(payload) {
        Ok(value) => Ok(value),
        Err(source) => Err(InvokeStreamError::Json { source }),
    }
}

type Pending = VecDeque<Result<InvokeChunk, InvokeStreamError>>;

pub(crate) struct FramedState<'decoder> {
    pub(crate) decoder: &'decoder mut InvokeStreamDecoder,
    pub(crate) buffer: BytesMut,
    pub(crate) tally: EventStreamTally,
    pub(crate) shape: ResponseStreamShape,
    pub(crate) response_headers: Option<BTreeMap<String, String>>,
    pub(crate) done: bool,
}

impl FramedState<'_> {
    pub(crate) fn absorb(&mut self, chunk: &[u8], pending: &mut Pending) {
        self.buffer.extend_from_slice(chunk);
        self.tally.add_chunk(chunk);
        loop {
            match decode_aws_event_frame(&mut self.buffer) {
                Ok(Some(event)) => {
                    self.tally.add_event(&event);
                    if !self.project(&event, pending) {
                        return;
                    }
                }
                Ok(None) => return,
                Err(source) => {
                    pending.push_back(Err(InvokeStreamError::Frame(source)));
                    self.done = true;
                    return;
                }
            }
        }
    }

    fn project(&mut self, event: &AwsEvent, pending: &mut Pending) -> bool {
        let message = match parse_message_from_event(Some(&self.shape), event) {
            Ok(Some(text)) => text,
            Ok(None) => return true,
            Err(source) => {
                pending.push_back(Err(InvokeStreamError::Frame(source)));
                self.done = true;
                return false;
            }
        };
        if message.is_empty() {
            return true;
        }
        let projected = decode_bedrock_event_json(message.as_bytes())
            .and_then(|chunk| self.decoder.chunk_parser(&chunk));
        match projected {
            Ok(chunk) => {
                pending.push_back(Ok(chunk));
                true
            }
            Err(error) => {
                pending.push_back(Err(error));
                self.done = true;
                false
            }
        }
    }

    pub(crate) fn finish(&mut self, pending: &mut Pending, redactor: &Redactor) {
        self.done = true;
        let headers = self.response_headers.as_ref();
        let error = self.tally.undecoded_stream_error(headers, redactor);
        pending.extend(error.map(|error| Err(InvokeStreamError::Frame(error))));
    }
}

pub struct FramedEvents<'decoder, Chunks> {
    pub(crate) state: FramedState<'decoder>,
    pub(crate) chunks: Chunks,
    pub(crate) pending: Pending,
    pub(crate) redactor: &'decoder Redactor,
}

impl<Chunks> Iterator for FramedEvents<'_, Chunks>
where
    Chunks: Iterator<Item = Bytes>,
{
    type Item = Result<InvokeChunk, InvokeStreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Some(item);
            }
            if self.state.done {
                return None;
            }
            match self.chunks.next() {
                Some(chunk) => self.state.absorb(&chunk, &mut self.pending),
                None => self.state.finish(&mut self.pending, self.redactor),
            }
        }
    }
}

impl<Chunks> Stream for FramedEvents<'_, Chunks>
where
    Chunks: Stream<Item = Bytes> + Unpin,
{
    type Item = Result<InvokeChunk, InvokeStreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.pending.pop_front() {
                return Poll::Ready(Some(item));
            }
            if this.state.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut this.chunks).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(chunk)) => this.state.absorb(&chunk, &mut this.pending),
                Poll::Ready(None) => this.state.finish(&mut this.pending, this.redactor),
            }
        }
    }
}
