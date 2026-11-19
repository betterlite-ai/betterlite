use std::collections::{BTreeMap, VecDeque};
use std::error::Error as StdError;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::mem;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::api::messages_stream_logging::seams::ReportedFailure;
use betterlite_core::api::messages_stream_logging::{
    ChunkError, SseChunk, convert_chunk_to_sse_format,
};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use bytes::BytesMut;
use futures_util::StreamExt as _;
use futures_util::stream::empty;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::agentcore_stream::transport::Chunks;
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::error::EventFrameError;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;
use crate::message_usage::{
    MessageUsageError, MessageUsageState, StreamItem, promote_invocation_metrics,
};

const BEDROCK_ERROR: &str = "BedrockError";
const CHUNK_ERROR: &str = "ChunkError";
const FRAME_ERROR: &str = "EventFrameError";
const JSON_ERROR: &str = "JSONDecodeError";
const MESSAGE_USAGE_ERROR: &str = "MessageUsageError";
const READ_ERROR: &str = "ReadError";

// Every failure keeps the typed cause it wraps, so a caller can still name what went wrong; the boxes keep the variants within three times each other.
#[derive(Debug, Error)]
enum MessagesStreamCause {
    #[error("{0}")]
    Frame(#[source] Box<EventFrameError>),
    #[error("{0}")]
    Json(#[source] Box<serde_json::Error>),
    #[error("{0}")]
    Usage(#[source] Box<MessageUsageError>),
    #[error("{0}")]
    Chunk(#[source] Box<ChunkError>),
    #[error("{0}")]
    Read(#[source] Box<HttpError>),
}

// The relay clones every failure it reports and queues, so the cause travels behind an Arc.
#[derive(Debug, Clone)]
struct SharedCause(Arc<MessagesStreamCause>);

impl Display for SharedCause {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        Display::fmt(&*self.0, formatter)
    }
}

impl StdError for SharedCause {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.0)
    }
}

// What the client sees when the stream dies: LiteLLM's exception class, its message, its status and the typed cause behind them.
#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct MessagesStreamError {
    pub kind: String,
    pub message: String,
    pub status: Option<u16>,
    #[source]
    cause: SharedCause,
}

// A Bedrock error event carries B682's error, so its class and status survive to the client.
impl From<EventFrameError> for MessagesStreamError {
    fn from(source: EventFrameError) -> Self {
        let (kind, message, status) = match &source {
            EventFrameError::Bedrock {
                status,
                error: bedrock,
            } => (BEDROCK_ERROR, bedrock.error.message.clone(), Some(*status)),
            EventFrameError::PreludeChecksum { .. }
            | EventFrameError::Frame(_)
            | EventFrameError::DuplicateHeader { .. }
            | EventFrameError::NotUtf8(_)
            | EventFrameError::NotJson(_)
            | EventFrameError::PayloadNotObject
            | EventFrameError::ChunkBytesNotText
            | EventFrameError::ChunkBase64(_)
            | EventFrameError::Header(_) => (FRAME_ERROR, source.to_string(), None),
        };
        Self {
            kind: kind.to_owned(),
            message,
            status,
            cause: SharedCause(Arc::new(MessagesStreamCause::Frame(Box::new(source)))),
        }
    }
}

impl From<serde_json::Error> for MessagesStreamError {
    fn from(source: serde_json::Error) -> Self {
        let message = source.to_string();
        Self {
            kind: JSON_ERROR.to_owned(),
            message,
            status: None,
            cause: SharedCause(Arc::new(MessagesStreamCause::Json(Box::new(source)))),
        }
    }
}

impl From<MessageUsageError> for MessagesStreamError {
    fn from(source: MessageUsageError) -> Self {
        let message = source.to_string();
        Self {
            kind: MESSAGE_USAGE_ERROR.to_owned(),
            message,
            status: None,
            cause: SharedCause(Arc::new(MessagesStreamCause::Usage(Box::new(source)))),
        }
    }
}

impl From<ChunkError> for MessagesStreamError {
    fn from(source: ChunkError) -> Self {
        let message = source.to_string();
        Self {
            kind: CHUNK_ERROR.to_owned(),
            message,
            status: None,
            cause: SharedCause(Arc::new(MessagesStreamCause::Chunk(Box::new(source)))),
        }
    }
}

impl From<HttpError> for MessagesStreamError {
    fn from(source: HttpError) -> Self {
        let message = source.to_string();
        Self {
            kind: READ_ERROR.to_owned(),
            message,
            status: None,
            cause: SharedCause(Arc::new(MessagesStreamCause::Read(Box::new(source)))),
        }
    }
}

impl From<MessagesStreamError> for ReportedFailure {
    fn from(error: MessagesStreamError) -> Self {
        Self {
            kind: error.kind,
            message: error.message,
        }
    }
}

pub struct MessagesStreamRequest {
    pub response_headers: Option<BTreeMap<String, String>>,
    pub redactor: Arc<Redactor>,
}

// get_async_streaming_response_iterator's iterator: the B609 decoder over the response body, B623's usage promotion, and the B979 SSE rendering `bedrock_sse_wrapper` yields.
pub struct MessagesSseStream {
    chunks: Chunks,
    buffer: BytesMut,
    tally: EventStreamTally,
    shape: ResponseStreamShape,
    headers: Option<BTreeMap<String, String>>,
    redactor: Arc<Redactor>,
    usage: MessageUsageState,
    pending: VecDeque<Vec<u8>>,
    failure: Option<MessagesStreamError>,
    ended: bool,
}

#[must_use]
pub fn messages_sse_stream(request: MessagesStreamRequest, chunks: Chunks) -> MessagesSseStream {
    MessagesSseStream {
        chunks,
        buffer: BytesMut::new(),
        tally: EventStreamTally::default(),
        shape: ResponseStreamShape::bedrock_runtime(),
        headers: request.response_headers,
        redactor: request.redactor,
        usage: MessageUsageState::default(),
        pending: VecDeque::new(),
        failure: None,
        ended: false,
    }
}

impl MessagesSseStream {
    // aiter_bytes: every complete frame in this read becomes one promoted, SSE-rendered chunk.
    fn absorb(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        self.tally.add_chunk(chunk);
        loop {
            let event = match decode_aws_event_frame(&mut self.buffer) {
                Ok(Some(event)) => event,
                Ok(None) => return,
                Err(error) => {
                    self.failure = Some(MessagesStreamError::from(error));
                    return;
                }
            };
            self.tally.add_event(&event);
            let message = match parse_message_from_event(Some(&self.shape), &event) {
                Ok(message) => message,
                Err(error) => {
                    self.failure = Some(MessagesStreamError::from(error));
                    return;
                }
            };
            let Some(text) = message else {
                continue;
            };
            if let Err(failure) = self.promote(&text) {
                self.failure = Some(failure);
                return;
            }
        }
    }

    // json.loads, the _chunk_parser metrics merge, the promotion state, then one SSE frame per item.
    fn promote(&mut self, text: &str) -> Result<(), MessagesStreamError> {
        let raw: Box<RawValue> = serde_json::from_str(text)?;
        let item = StreamItem::try_from(raw)?;
        let promoted = match item {
            StreamItem::Event(fields) => StreamItem::Event(promote_invocation_metrics(fields)?),
            StreamItem::Chunk(chunk) => StreamItem::Chunk(chunk),
        };
        let mut items = Vec::new();
        self.usage.push(promoted, &mut items)?;
        for rendered in items {
            self.render(rendered)?;
        }
        Ok(())
    }

    fn render(&mut self, item: StreamItem) -> Result<(), MessagesStreamError> {
        let encoded = match item {
            StreamItem::Event(fields) => {
                let mut mapping = IndexMap::with_capacity(fields.len());
                for (name, value) in fields {
                    let parsed: Json = serde_json::from_str(value.get())?;
                    mapping.insert(name, parsed);
                }
                convert_chunk_to_sse_format(SseChunk::Mapping(mapping))?
            }
            // _convert_chunk_to_sse_format returns a non-dict chunk as it is.
            StreamItem::Chunk(raw) => raw.get().as_bytes().to_vec(),
        };
        self.pending.push_back(encoded);
        Ok(())
    }

    // The tally runs before the promotion flush, so a cut body never yields its pending delta.
    fn finish(&mut self) {
        if let Some(error) = self
            .tally
            .undecoded_stream_error(self.headers.as_ref(), &self.redactor)
        {
            self.failure = Some(MessagesStreamError::from(error));
            return;
        }
        let mut items = Vec::new();
        let usage = mem::take(&mut self.usage);
        if let Err(error) = usage.finish(&mut items) {
            self.failure = Some(MessagesStreamError::from(error));
            return;
        }
        for item in items {
            if let Err(failure) = self.render(item) {
                self.failure = Some(failure);
                return;
            }
        }
    }
}

#[async_trait]
impl MessagesProviderStream for MessagesSseStream {
    type Error = MessagesStreamError;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        loop {
            if let Some(encoded) = self.pending.pop_front() {
                return Ok(Some(encoded));
            }
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }
            if self.ended {
                return Ok(None);
            }
            match self.chunks.next().await {
                Some(Ok(chunk)) => self.absorb(&chunk),
                Some(Err(error)) => {
                    self.ended = true;
                    self.failure = Some(MessagesStreamError::from(error));
                }
                None => {
                    self.ended = true;
                    self.finish();
                }
            }
        }
    }

    // bedrock_sse_wrapper's finally closes the sse_stream; here that releases the response body.
    async fn aclose(&mut self) -> Option<Result<(), Self::Error>> {
        self.ended = true;
        self.pending.clear();
        self.chunks = Box::pin(empty::<Result<Vec<u8>, HttpError>>());
        Some(Ok(()))
    }
}
