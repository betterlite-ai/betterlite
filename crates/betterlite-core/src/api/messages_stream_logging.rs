use std::sync::{Arc, PoisonError};

use betterlite_logging::logging_state::events::update_completion_start_time;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::python_json::dumps_object;
use betterlite_logging::worker::{Admission, LoggingWorker};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

use super::messages_sse_events::stream::MessagesProviderStream;
use seams::{ChunkProcessorCall, LoggingJob, ReportedFailure, RouteStreamingLogging, StreamSeams};
use slots::StreamSlots;

pub mod seams;
pub mod slots;

mod tests;

// Bedrock streams log under the same literal route and endpoint type
const MESSAGES_ROUTE: &str = "/v1/messages";

pub struct StreamDeps<S> {
    pub seams: Arc<S>,
    pub worker: Arc<LoggingWorker<LoggingJob>>,
    pub slots: Arc<StreamSlots>,
    pub clock: Arc<dyn SuccessClock>,
    pub success_handler: Arc<PassThroughEndpointLogging>,
}

pub struct MessagesStreamLogging<S> {
    deps: StreamDeps<S>,
    logging_obj: Arc<StreamLoggingObject>,
    request_body: IndexMap<String, Json>,
    start_time: DateTime<FixedOffset>,
    completion_start_time: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logged {
    Parked,
    Enqueued(Admission),
}

pub enum SseChunk {
    Mapping(IndexMap<String, Json>),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChunkError {
    #[error("the chunk's type must be text or null")]
    EventTypeNotText,
}

impl<S: StreamSeams> MessagesStreamLogging<S> {
    // The LiteLLM iterator's __init__ stamps the start time as it builds the stream
    pub fn new(
        deps: StreamDeps<S>,
        litellm_logging_obj: Arc<StreamLoggingObject>,
        request_body: IndexMap<String, Json>,
    ) -> Self {
        Self {
            start_time: deps.clock.now(),
            deps,
            logging_obj: litellm_logging_obj,
            request_body,
            completion_start_time: None,
        }
    }

    // B979's pump calls this for every chunk; only the first one stamps the time
    pub fn record_first_chunk(&mut self) {
        if self.completion_start_time.is_none() {
            self.completion_start_time = Some(self.deps.clock.now());
        }
    }

    pub fn handle_streaming_logging(
        &self,
        collected_chunks: Vec<Vec<u8>>,
        stream_teardown: bool,
    ) -> Result<Logged, ReportedFailure> {
        let end_time = self.deps.clock.now();
        if let Some(first_chunk) = self.completion_start_time {
            self.logging_obj
                .edit(&mut |state| update_completion_start_time(state, first_chunk));
        }
        let logging = self
            .deps
            .seams
            .route_streaming_logging(RouteStreamingLogging {
                logging_obj: Arc::clone(&self.logging_obj),
                success_handler: Arc::clone(&self.deps.success_handler),
                url_route: MESSAGES_ROUTE,
                request_body: self.request_body.clone(),
                endpoint_type: EndpointType::Anthropic,
                start_time: self.start_time,
                raw_bytes: collected_chunks,
                end_time,
            })?;
        let mut fields = self
            .deps
            .slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if fields.deferred_callback.is_some() && !stream_teardown {
            fields.deferred_args = Some(vec![logging]);
            return Ok(Logged::Parked);
        }
        drop(fields);
        // the rooted worker, since an unrooted task could be dropped before it bills during teardown
        Ok(Logged::Enqueued(
            self.deps.worker.enqueue(LoggingJob { future: logging }),
        ))
    }

    pub fn bill_collected_chunks(
        &self,
        collected_chunks: Vec<Vec<u8>>,
        stream_teardown: bool,
    ) -> Option<Logged> {
        let count = collected_chunks.len();
        match self.handle_streaming_logging(collected_chunks, stream_teardown) {
            Ok(logged) => Some(logged),
            Err(failure) => {
                tracing::warn!("async_sse_wrapper billing failed after {count} chunks: {failure}");
                None
            }
        }
    }

    pub fn dispatch_pending_logging(&self) -> Option<Admission> {
        let (callback, args) = self.deps.slots.take_pending()?;
        let logging = callback(args);
        Some(self.deps.worker.enqueue(LoggingJob { future: logging }))
    }

    pub async fn fire_detached_failure_hook(&self, exception: &ReportedFailure) {
        let hook = self
            .deps
            .slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .detached_failure_hook
            .clone();
        let Some(hook) = hook else {
            return;
        };
        if let Err(failure) = hook.on_detached_failure(exception).await {
            tracing::warn!("async_sse_wrapper detached failure hook raised: {failure}");
        }
    }

    // B3609 relays the response and logs it; this leaf installs no second relay
    pub fn streaming_response_iterator(
        &self,
        response: S::Response,
        request_body: IndexMap<String, Json>,
        logging_obj: Arc<StreamLoggingObject>,
    ) -> S::Stream {
        self.deps.seams.chunk_processor(ChunkProcessorCall {
            response,
            request_body,
            logging_obj,
            endpoint_type: EndpointType::Anthropic,
            start_time: self.start_time,
            success_handler: Arc::clone(&self.deps.success_handler),
            url_route: MESSAGES_ROUTE,
        })
    }
}

pub async fn abort_upstream<P>(completion_stream: &mut P)
where
    P: MessagesProviderStream + Send,
    ReportedFailure: From<P::Error>,
{
    if let Some(Err(error)) = completion_stream.aclose().await {
        let failure = ReportedFailure::from(error);
        tracing::warn!("async_sse_wrapper failed to abort upstream stream: {failure}");
    }
}

pub fn convert_chunk_to_sse_format(chunk: SseChunk) -> Result<Vec<u8>, ChunkError> {
    match chunk {
        SseChunk::Bytes(bytes) => Ok(bytes),
        SseChunk::Mapping(mapping) => {
            let event_type = match mapping.get("type") {
                None => "message",
                Some(Json::Null) => "None",
                Some(Json::Text(text)) => text,
                Some(_) => return Err(ChunkError::EventTypeNotText),
            };
            let payload = format!("event: {event_type}\ndata: {}\n\n", dumps_object(&mapping));
            Ok(payload.into_bytes())
        }
    }
}
