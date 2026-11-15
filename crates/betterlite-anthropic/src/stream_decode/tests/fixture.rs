use std::cell::Cell;
use std::convert::Infallible;
use std::fmt::Debug;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use futures_util::{Stream, StreamExt as _, stream};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use uuid::Uuid;

use crate::stream_decode::error::{DecodeError, ReceiveError};
use crate::stream_decode::passthrough::{
    ConvertError, ConvertedChunk, convert_chunk_to_generic_chunk,
};
use crate::stream_decode::{ChunkStream, DecodedChunk, TransportChunk};
use crate::stream_events::StreamEventConverter;
use crate::stream_events::error::StreamEventError;
use crate::stream_state::state::{IdSource, StreamOptions, StreamState};
use crate::usage::ReasoningTokenCounter;

pub(super) const RESPONSE_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
pub(super) const RESPONSE_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";
// The shared ApiLimits default max_event_bytes.
pub(super) const MAX_EVENT_BYTES: usize = 0x0010_0000;

// A source is pending once before every item, so the Stream implementation and cancellation run for real.
pub(super) struct PendFirst<S> {
    pub(super) inner: S,
    pub(super) pending: bool,
}

impl<S, Item> Stream for PendFirst<S>
where
    S: Stream<Item = Item> + Unpin,
{
    type Item = Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Item>> {
        let this = self.get_mut();
        if this.pending {
            this.pending = false;
            return Pin::new(&mut this.inner).poll_next(cx);
        }
        this.pending = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

// Counts the items the inner source yielded and records its drop, so a test can see what a cancel pulls.
pub(super) struct Tracked<S> {
    pub(super) inner: S,
    pub(super) yielded: Rc<Cell<usize>>,
    pub(super) dropped: Rc<Cell<bool>>,
}

impl<S, Item> Stream for Tracked<S>
where
    S: Stream<Item = Item> + Unpin,
{
    type Item = Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Item>> {
        let this = self.get_mut();
        let polled = this.inner.poll_next_unpin(cx);
        let yielded = usize::from(polled.is_ready());
        this.yielded.set(this.yielded.get().saturating_add(yielded));
        polled
    }
}

impl<S> Drop for Tracked<S> {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Warnings {
    pub(super) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture_warnings() -> (DefaultGuard, Warnings) {
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    (set_default(subscriber), warnings)
}

struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(super) struct Counter;

impl ReasoningTokenCounter for Counter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub(super) struct SourceFailure {
    pub(super) message: String,
}

pub(super) type Source = Result<TransportChunk, ReceiveError<SourceFailure>>;
pub(super) type Outcome = Result<DecodedChunk, DecodeError<SourceFailure, Infallible>>;
pub(super) type Conversion = Result<ConvertedChunk, ConvertError<Infallible>>;

pub(super) fn delta(label: &str) -> String {
    format!(
        r#"data:{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{label}"}}}}"#
    )
}

pub(super) fn converter() -> StreamEventConverter<Counter> {
    let ids = FixedIds(Uuid::from_u128(RESPONSE_UUID));
    let state = StreamState::new(
        StreamOptions {
            sync_stream: false,
            json_mode: None,
            speed: None,
            tool_name_reverse_map: IndexMap::new(),
        },
        &ids,
    );
    StreamEventConverter {
        state,
        counter: Counter,
        json_could_close: false,
    }
}

pub(super) fn drive_sync(items: Vec<Source>) -> Vec<Outcome> {
    ChunkStream {
        converter: converter(),
        source: items.into_iter(),
        source_ended: false,
        max_event_bytes: MAX_EVENT_BYTES,
    }
    .collect()
}

pub(super) fn drive_async(items: Vec<Source>) -> Vec<Outcome> {
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let mut stream = ChunkStream {
                converter: converter(),
                source: PendFirst {
                    inner: stream::iter(items),
                    pending: false,
                },
                source_ended: false,
                max_event_bytes: MAX_EVENT_BYTES,
            };
            let mut outcomes = Vec::new();
            while let Some(outcome) = stream.next().await {
                outcomes.push(outcome);
            }
            outcomes
        })
}

pub(super) fn convert(chunks: Vec<TransportChunk>) -> Vec<Conversion> {
    let mut converter = converter();
    chunks
        .into_iter()
        .map(|chunk| convert_chunk_to_generic_chunk(&mut converter, chunk))
        .collect()
}

pub(super) fn generic(outcome: &Outcome) -> &GenericChunk {
    match outcome {
        Ok(DecodedChunk::Generic(chunk)) => chunk,
        Ok(DecodedChunk::Event(_)) => panic!("a generic chunk was expected, not an event"),
        Err(error) => panic!("a generic chunk was expected, got the error {error}"),
    }
}

pub(super) fn failure(outcome: &Outcome) -> &DecodeError<SourceFailure, Infallible> {
    match outcome {
        Err(error) => error,
        Ok(DecodedChunk::Event(_) | DecodedChunk::Generic(_)) => panic!("an error was expected"),
    }
}

pub(super) fn stream_failure(
    error: &DecodeError<SourceFailure, Infallible>,
) -> &StreamEventError<Infallible> {
    match error {
        DecodeError::Event(inner) => inner,
        DecodeError::Receiving { source } | DecodeError::Other { source } => {
            panic!("an event error was expected, got {source}")
        }
        DecodeError::Parsing { source, .. } => panic!("an event error was expected, got {source}"),
        DecodeError::TooLarge { limit } => {
            panic!("an event error was expected, got the {limit}-byte event limit")
        }
    }
}
