use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_http::error::HttpError;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use bytes::BytesMut;
use futures_util::Stream;
use http::HeaderMap;
use thiserror::Error;

use crate::agentcore_stream::adapter::header_pairs;
use crate::agentcore_stream::transport::{Chunks, StreamHead};
use crate::bedrock_http_stream::StreamDeps;
use crate::converse_events::ChunkProjection;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::fake::FakeStream;
use crate::invoke_events::framing::FramedState;
use crate::invoke_events::{DecoderOptions, InvokeChunk, InvokeStreamDecoder, stream_decoder};

type Pending = VecDeque<Result<InvokeChunk, InvokeStreamError>>;

// What a helper hands back: the iterator a consumer reads and the response headers LiteLLM returns beside it.
pub struct OpenedStream<'deps> {
    pub chunks: ChunkStream<'deps>,
    pub headers: HeaderMap,
}

// LiteLLM returns a sync iterator from make_sync_call and an async one from make_call; one stream serves both.
pub enum ChunkStream<'deps> {
    Framed(Box<FramedStream<'deps>>),
    Fake(FakeStream),
}

// A body read that fails ends the stream and is never retried; a decode failure is the provider's answer.
#[derive(Debug, Error)]
pub enum StreamFailure {
    #[error("the response body could not be read")]
    Read(#[source] HttpError),
    #[error(transparent)]
    Decode(#[from] InvokeStreamError),
}

impl StreamFailure {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Read(_) => None,
            Self::Decode(error) => error.status(),
        }
    }
}

impl Stream for ChunkStream<'_> {
    type Item = Result<InvokeChunk, StreamFailure>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut() {
            Self::Framed(stream) => Pin::new(stream.as_mut()).poll_next(cx),
            Self::Fake(stream) => Poll::Ready(stream.next().map(fake_item)),
        }
    }
}

// MockResponseIterator is immediate: one chunk or one failure, then it is done.
fn fake_item(item: Result<GenericChunk, InvokeStreamError>) -> Result<InvokeChunk, StreamFailure> {
    match item {
        Ok(chunk) => Ok(InvokeChunk::Base(Box::new(ChunkProjection::Generic(chunk)))),
        Err(source) => Err(StreamFailure::Decode(source)),
    }
}

pub struct FramedStream<'deps> {
    decoder: InvokeStreamDecoder,
    chunks: Chunks,
    framing: Framing,
    redactor: &'deps Redactor,
    pending: Pending,
    read_failure: Option<HttpError>,
}

impl FramedStream<'_> {
    fn read_next(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<InvokeChunk, StreamFailure>>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                return Poll::Ready(Some(item.map_err(StreamFailure::Decode)));
            }
            if let Some(failure) = self.read_failure.take() {
                return Poll::Ready(Some(Err(StreamFailure::Read(failure))));
            }
            if self.framing.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut self.chunks).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => {
                    self.framing
                        .absorb(&mut self.decoder, &mut self.pending, &chunk);
                }
                // LiteLLM's loop over the response ends at the exception, so the tally never reports and nothing retries.
                Poll::Ready(Some(Err(failure))) => {
                    self.framing.done = true;
                    self.read_failure = Some(failure);
                }
                Poll::Ready(None) => {
                    // The body is spent here whether or not the tally reports it, so mark it first.
                    self.framing.done = true;
                    self.framing
                        .finish(&mut self.decoder, &mut self.pending, self.redactor);
                }
            }
        }
    }
}

impl Stream for FramedStream<'_> {
    type Item = Result<InvokeChunk, StreamFailure>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().read_next(cx)
    }
}

// The decoder construction make_call makes for a framed answer: the provider picks the projection, sync the flag.
pub struct FramedCall<'call> {
    pub model: &'call str,
    pub json_mode: Option<bool>,
    pub provider: Option<&'call str>,
    pub sync: bool,
}

#[must_use]
pub fn framed<'deps>(
    call: &FramedCall<'_>,
    deps: &StreamDeps<'deps>,
    head: StreamHead,
) -> OpenedStream<'deps> {
    let options = DecoderOptions {
        model: call.model,
        json_mode: call.json_mode,
        sync_stream: call.sync,
        clock: deps.clock,
    };
    let headers: BTreeMap<String, String> = header_pairs(&head.headers).into_iter().collect();
    OpenedStream {
        headers: head.headers.clone(),
        chunks: ChunkStream::Framed(Box::new(FramedStream {
            decoder: stream_decoder(call.provider, &options),
            chunks: head.chunks,
            framing: Framing {
                buffer: BytesMut::new(),
                tally: EventStreamTally::default(),
                shape: Some(ResponseStreamShape::bedrock_runtime()),
                headers: Some(headers),
                done: false,
            },
            redactor: deps.redactor,
            pending: Pending::new(),
            read_failure: None,
        })),
    }
}

// B599's frame state owns its decoder borrow and its counters, so each call carries them in and takes them back.
struct Framing {
    buffer: BytesMut,
    tally: EventStreamTally,
    // Held as an option only so the state can borrow it by value; the map is built once per stream.
    shape: Option<ResponseStreamShape>,
    headers: Option<BTreeMap<String, String>>,
    done: bool,
}

impl Framing {
    fn absorb(&mut self, decoder: &mut InvokeStreamDecoder, pending: &mut Pending, chunk: &[u8]) {
        let mut state = self.state(decoder);
        state.absorb(chunk, pending);
        self.collect(state);
    }

    fn finish(
        &mut self,
        decoder: &mut InvokeStreamDecoder,
        pending: &mut Pending,
        redactor: &Redactor,
    ) {
        let state = self.state(decoder);
        state.finish(pending, redactor);
        self.collect(state);
    }

    fn state<'state>(&mut self, decoder: &'state mut InvokeStreamDecoder) -> FramedState<'state> {
        FramedState {
            decoder,
            buffer: mem::take(&mut self.buffer),
            tally: mem::take(&mut self.tally),
            shape: self
                .shape
                .take()
                .unwrap_or_else(ResponseStreamShape::bedrock_runtime),
            response_headers: self.headers.clone(),
            done: self.done,
        }
    }

    fn collect(&mut self, state: FramedState<'_>) {
        self.buffer = state.buffer;
        self.tally = state.tally;
        self.shape = Some(state.shape);
        self.done = state.done;
    }
}
