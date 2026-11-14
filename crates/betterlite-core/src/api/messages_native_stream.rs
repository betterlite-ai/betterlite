use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::messages_native_stream::Settings;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use super::api_boundaries::ApiLimits;
use super::messages_sse_events::chunk::ChunkFlag;
use super::messages_sse_events::decode_messages_sse_chunk;
use super::messages_sse_events::stream::{MessagesProviderStream, MessagesStreamingResponse};
use super::messages_stream_logging::seams::{ReportedFailure, StreamSeams};
use super::messages_stream_logging::{MessagesStreamLogging, SseChunk, StreamDeps};
use pump::Pump;
use queue::{ClientQueue, QueueItem};
use registry::StreamRegistry;
use retention::RetentionBudget;

mod pump;
mod queue;
pub mod registry;
pub mod retention;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayLimits {
    pub queue_maxsize: usize,
    pub max_detached_stream_drains: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RelayConfigError {
    // LiteLLM makes a zero or negative queue unbounded; a bounded relay cannot
    #[error("ANTHROPIC_MESSAGES_STREAM_RELAY_QUEUE_MAXSIZE must be greater than 0")]
    NonPositiveQueue,
}

impl RelayLimits {
    pub fn from_settings(settings: &Settings) -> Result<Self, RelayConfigError> {
        let queue_maxsize = usize::try_from(settings.queue_maxsize)
            .ok()
            .filter(|size| *size > 0)
            .ok_or(RelayConfigError::NonPositiveQueue)?;
        // The source comparison makes zero and negative caps disable detached draining
        let max_detached_stream_drains =
            usize::try_from(settings.max_detached_stream_drains).unwrap_or_default();
        Ok(Self {
            queue_maxsize,
            max_detached_stream_drains,
        })
    }
}

// B3609's failure logging as the relay injects it, with the request body, partial bytes and exception
pub struct StreamFailureCall {
    pub request_body: IndexMap<String, Json>,
    pub endpoint: EndpointType,
    pub failure: ReportedFailure,
    pub raw_bytes: Vec<Vec<u8>>,
}

#[async_trait]
pub trait StreamFailureLogging: Send + Sync {
    async fn schedule_failure_logging(&self, call: StreamFailureCall);
}

// A source hands the relay encoded chunks or decoded mappings, so both follow B2436's format
#[async_trait]
pub trait MessagesChunkSource: MessagesProviderStream {
    async fn next_sse_chunk(&mut self) -> Result<Option<SseChunk>, Self::Error> {
        MessagesProviderStream::next_chunk(self)
            .await
            .map(|chunk| chunk.map(SseChunk::Bytes))
    }
}

impl<S: MessagesProviderStream + Send> MessagesChunkSource for MessagesStreamingResponse<S> {}

pub struct RelaySetup<S, F> {
    pub logging: StreamDeps<S>,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub request_body: IndexMap<String, Json>,
    pub failure_logging: Arc<F>,
    pub registry: Arc<StreamRegistry>,
    pub limits: RelayLimits,
    pub boundaries: ApiLimits,
}

pub struct ClientRelay<S: StreamSeams, E> {
    queue: Arc<ClientQueue<E>>,
    teardown: MessagesStreamLogging<S>,
    reached_end: bool,
}

// The client relay plus the tracker a teardown waits on after dropping it; dropping never aborts
pub struct RelayHandle<S: StreamSeams, E> {
    pub relay: ClientRelay<S, E>,
    pub pump: Arc<TaskTracker>,
}

impl<S: StreamSeams, E> ClientRelay<S, E> {
    pub async fn next_client_chunk(&mut self) -> Result<Option<Vec<u8>>, Arc<E>>
    where
        E: Send + Sync,
    {
        match self.queue.next().await {
            Some(QueueItem::Chunk(bytes)) => Ok(Some(bytes)),
            Some(QueueItem::End) => {
                self.reached_end = true;
                Ok(None)
            }
            Some(QueueItem::Failed(error)) => Err(error),
            // the pump ended without a sentinel; the relay treats it as a torn-down stream
            None => Ok(None),
        }
    }
}

impl<S: StreamSeams, E> Drop for ClientRelay<S, E> {
    fn drop(&mut self) {
        self.queue.detach();
        if !self.reached_end {
            let _dispatched = self.teardown.dispatch_pending_logging();
        }
    }
}

#[must_use]
pub fn start_relay<P, S, F>(stream: P, setup: RelaySetup<S, F>) -> RelayHandle<S, P::Error>
where
    P: MessagesChunkSource + Send + 'static,
    P::Error: Clone + Send + Sync + 'static,
    S: StreamSeams + 'static,
    F: StreamFailureLogging + 'static,
    ReportedFailure: From<P::Error>,
{
    let RelaySetup {
        logging,
        logging_obj,
        request_body,
        failure_logging,
        registry,
        limits,
        boundaries,
    } = setup;
    let teardown = teardown_logging(&logging, Arc::clone(&logging_obj), &request_body);
    let pump = relay_pump(
        MessagesStreamLogging::new(logging, logging_obj, request_body.clone()),
        failure_logging,
        request_body,
        boundaries,
        limits,
    );
    let queue = Arc::new(ClientQueue {
        capacity: limits.queue_maxsize,
        state: Mutex::new(queue::State {
            items: VecDeque::new(),
            closed: false,
        }),
        data: Notify::new(),
        space: Notify::new(),
        client_detached: AtomicBool::new(false),
        detach: Notify::new(),
    });
    spawn_relay(stream, pump, queue, registry, teardown)
}

// The teardown copy shares the pump's logger and slots, so a lost sentinel can still dispatch
fn teardown_logging<S>(
    logging: &StreamDeps<S>,
    logging_obj: Arc<StreamLoggingObject>,
    request_body: &IndexMap<String, Json>,
) -> MessagesStreamLogging<S>
where
    S: StreamSeams,
{
    MessagesStreamLogging::new(
        StreamDeps {
            seams: Arc::clone(&logging.seams),
            worker: Arc::clone(&logging.worker),
            slots: Arc::clone(&logging.slots),
            clock: Arc::clone(&logging.clock),
            success_handler: Arc::clone(&logging.success_handler),
        },
        logging_obj,
        request_body.clone(),
    )
}

const fn relay_pump<S, F>(
    logging: MessagesStreamLogging<S>,
    failure_logging: Arc<F>,
    request_body: IndexMap<String, Json>,
    boundaries: ApiLimits,
    limits: RelayLimits,
) -> Pump<S, F>
where
    S: StreamSeams,
    F: StreamFailureLogging,
{
    let budget = RetentionBudget {
        max_bytes: boundaries.max_json_bytes,
        max_chunks: boundaries.max_items,
        bytes: 0,
        chunks: 0,
    };
    Pump {
        logging,
        failure_logging,
        request_body,
        boundaries,
        max_detached_stream_drains: limits.max_detached_stream_drains,
        collected: Vec::new(),
        budget,
        saw_terminal: false,
        drain: None,
    }
}

fn spawn_relay<P, S, F>(
    stream: P,
    mut pump: Pump<S, F>,
    queue: Arc<ClientQueue<P::Error>>,
    registry: Arc<StreamRegistry>,
    teardown: MessagesStreamLogging<S>,
) -> RelayHandle<S, P::Error>
where
    P: MessagesChunkSource + Send + 'static,
    P::Error: Clone + Send + Sync + 'static,
    S: StreamSeams + 'static,
    F: StreamFailureLogging + 'static,
    ReportedFailure: From<P::Error>,
{
    let registration = registry.register_pump();
    let worker_queue = Arc::clone(&queue);
    let tracker = Arc::new(TaskTracker::new());
    tracker.spawn(async move {
        let _registration = registration;
        pump.run(stream, &worker_queue, &registry).await;
        worker_queue.close();
    });
    tracker.close();
    RelayHandle {
        relay: ClientRelay {
            queue,
            teardown,
            reached_end: false,
        },
        pump: tracker,
    }
}

// LiteLLM's _is_message_stop_chunk and _is_provider_error_chunk, which B3608 probes through its AnthropicFrameProbe trait
#[must_use]
pub fn is_message_stop_chunk(frames: &[u8]) -> bool {
    let Some(flags) = classified_flags(frames) else {
        return false;
    };
    flags.contains(&ChunkFlag::MessageStop)
}

#[must_use]
pub fn is_provider_error_chunk(frames: &[u8]) -> bool {
    let Some(flags) = classified_flags(frames) else {
        return false;
    };
    flags.contains(&ChunkFlag::ProviderError)
}

// The frames are already in memory, so the per-read byte bound must not reject a joined stream
fn classified_flags(frames: &[u8]) -> Option<BTreeSet<ChunkFlag>> {
    let defaults = ApiLimits::default();
    let limits = ApiLimits {
        max_json_bytes: defaults.max_json_bytes,
        max_event_bytes: frames.len(),
        max_items: defaults.max_items,
    };
    decode_messages_sse_chunk(frames, &limits)
        .ok()
        .map(|decoded| decoded.flags)
}
