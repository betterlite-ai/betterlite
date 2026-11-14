use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::messages_native_stream::Settings;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use super::api_boundaries::{ApiLimits, LimitErrorFields};
use super::messages_sse_events::stream::{MessagesProviderStream, MessagesStreamingResponse};
use super::messages_stream_logging::seams::{ReportedFailure, StreamSeams};
use super::messages_stream_logging::{MessagesStreamLogging, SseChunk, StreamDeps};
use pump::Pump;
use queue::{ClientQueue, QueueItem};
use registry::StreamRegistry;

mod pump;
mod queue;
pub mod registry;
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
            usize::try_from(settings.max_detached_stream_drains).unwrap_or(0);
        Ok(Self {
            queue_maxsize,
            max_detached_stream_drains,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Messages stream exceeded its retained-output limit")]
pub struct RetainedOutputLimit;

impl RetainedOutputLimit {
    pub const FIELDS: LimitErrorFields = LimitErrorFields {
        status: 502,
        error_type: "bad_gateway",
        param: "stream",
        code: "stream_buffer_too_large",
    };
}

// B1087's byte and item limits bound the encoded output kept for the billing call
pub struct RetentionBudget {
    max_bytes: usize,
    max_chunks: usize,
    bytes: usize,
    chunks: usize,
}

impl RetentionBudget {
    pub fn accept(&mut self, chunk_bytes: usize) -> Result<(), RetainedOutputLimit> {
        let bytes = self
            .bytes
            .checked_add(chunk_bytes)
            .ok_or(RetainedOutputLimit)?;
        let chunks = self.chunks.checked_add(1).ok_or(RetainedOutputLimit)?;
        if bytes > self.max_bytes || chunks > self.max_chunks {
            return Err(RetainedOutputLimit);
        }
        self.bytes = bytes;
        self.chunks = chunks;
        Ok(())
    }
}

// B3609's schedule_stream_failure_logging as the relay injects it
pub struct StreamFailureCall {
    pub request_body: IndexMap<String, Json>,
    pub endpoint: EndpointType,
    pub failure: ReportedFailure,
    pub raw_bytes: Vec<Vec<u8>>,
}

#[async_trait]
pub trait StreamFailureLogging: Send + Sync {
    async fn schedule_stream_failure_logging(&self, call: StreamFailureCall);
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
    // dropping the tracker never aborts the pump; a caller that waits holds this tracker
    pump: Arc<TaskTracker>,
}

impl<S: StreamSeams, E> ClientRelay<S, E> {
    pub async fn next_client_chunk(&mut self) -> Result<Option<Vec<u8>>, Arc<E>> {
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
pub fn start_relay<P, S, F>(stream: P, setup: RelaySetup<S, F>) -> ClientRelay<S, P::Error>
where
    P: MessagesChunkSource + Send + 'static,
    P::Error: Clone + Send + 'static,
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
    let teardown = MessagesStreamLogging::new(
        StreamDeps {
            seams: Arc::clone(&logging.seams),
            worker: Arc::clone(&logging.worker),
            slots: Arc::clone(&logging.slots),
            clock: Arc::clone(&logging.clock),
            success_handler: Arc::clone(&logging.success_handler),
        },
        Arc::clone(&logging_obj),
        request_body.clone(),
    );
    let pump = Pump {
        logging: MessagesStreamLogging::new(logging, logging_obj, request_body.clone()),
        failure_logging,
        request_body,
        boundaries,
        max_detached_stream_drains: limits.max_detached_stream_drains,
        collected: Vec::new(),
        budget: RetentionBudget {
            max_bytes: boundaries.max_json_bytes,
            max_chunks: boundaries.max_items,
            bytes: 0,
            chunks: 0,
        },
        saw_terminal: false,
        drain: None,
    };
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
    let registration = registry.start_pump();
    let worker_queue = Arc::clone(&queue);
    let tracker = Arc::new(TaskTracker::new());
    tracker.spawn(async move {
        let _registration = registration;
        pump.run(stream, &worker_queue, &registry).await;
        worker_queue.close();
    });
    tracker.close();
    ClientRelay {
        queue,
        teardown,
        reached_end: false,
        pump: tracker,
    }
}

#[must_use]
pub fn error_sse_event(message: &str) -> Vec<u8> {
    let error = Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("error".to_owned())),
        (
            "error".to_owned(),
            Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("api_error".to_owned())),
                ("message".to_owned(), Json::Text(message.to_owned())),
            ])),
        ),
    ]));
    format!("event: error\ndata: {}\n\n", dumps(&error)).into_bytes()
}
