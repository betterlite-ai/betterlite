use std::mem::take;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::queue::{ClientQueue, QueueItem};
use super::registry::{Slot, StreamRegistry};
use super::retention::RetentionBudget;
use super::{MessagesChunkSource, ReportedFailure, StreamFailureCall, StreamFailureLogging};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_sse_events::chunk::{ChunkFlag, DecodedMessagesChunk};
use crate::api::messages_sse_events::render::incomplete_error_sse_event;
use crate::api::messages_sse_events::{decode_messages_sse_chunk, decode_messages_sse_value};
use crate::api::messages_stream_logging::seams::StreamSeams;
use crate::api::messages_stream_logging::{
    MessagesStreamLogging, SseChunk, abort_upstream, convert_chunk_to_sse_format,
};
use crate::api::messages_stream_split::errors::{
    MessagesStreamSplitError, StreamFailure, mid_stream_error_sse_event,
};

enum Step {
    Continue,
    Stop,
    Abort,
}

pub(super) struct Pump<S, F> {
    pub(super) logging: MessagesStreamLogging<S>,
    pub(super) failure_logging: Arc<F>,
    pub(super) request_body: IndexMap<String, Json>,
    pub(super) boundaries: ApiLimits,
    pub(super) max_detached_stream_drains: usize,
    pub(super) collected: Vec<Vec<u8>>,
    pub(super) budget: RetentionBudget,
    pub(super) saw_terminal: bool,
    pub(super) drain: Option<Slot>,
}

impl<S, F> Pump<S, F>
where
    S: StreamSeams,
    F: StreamFailureLogging,
{
    pub(super) async fn run<P>(
        &mut self,
        mut stream: P,
        queue: &ClientQueue<P::Error>,
        registry: &Arc<StreamRegistry>,
    ) where
        P: MessagesChunkSource + Send,
        P::Error: Clone + Send + Sync + 'static,
        ReportedFailure: From<P::Error>,
    {
        loop {
            let step = match stream.next_sse_chunk().await {
                Ok(Some(chunk)) => self.process(chunk, queue, registry).await,
                Ok(None) => {
                    self.complete(queue).await;
                    Step::Stop
                }
                Err(error) => {
                    self.failed(queue, error).await;
                    Step::Stop
                }
            };
            match step {
                Step::Continue => {}
                Step::Stop => return,
                Step::Abort => {
                    abort_upstream(&mut stream).await;
                    return;
                }
            }
        }
    }

    async fn process<E>(
        &mut self,
        chunk: SseChunk,
        queue: &ClientQueue<E>,
        registry: &Arc<StreamRegistry>,
    ) -> Step
    where
        E: Send + Sync,
    {
        self.logging.record_first_chunk();
        let decoded = match Self::decode(&chunk, &self.boundaries) {
            Ok(decoded) => decoded,
            Err(failure) => return self.limit_reached(queue, failure).await,
        };
        self.saw_terminal |= decoded.flags.contains(&ChunkFlag::Terminal);
        let encoded = match convert_chunk_to_sse_format(chunk) {
            Ok(encoded) => encoded,
            Err(error) => {
                let failure = ReportedFailure {
                    kind: "ChunkError".to_owned(),
                    message: error.to_string(),
                };
                self.schedule_failure(failure).await;
                return Step::Stop;
            }
        };
        if let Err(limit) = self.budget.accept(encoded.len()) {
            let failure = ReportedFailure {
                kind: "BadGatewayError".to_owned(),
                message: limit.to_string(),
            };
            return self.limit_reached(queue, failure).await;
        }
        self.collected.push(encoded.clone());
        if queue.client_detached.load(Ordering::SeqCst) {
            return self.detached_chunk(registry);
        }
        let _queued = queue.enqueue(QueueItem::Chunk(encoded)).await;
        Step::Continue
    }

    fn decode(
        chunk: &SseChunk,
        boundaries: &ApiLimits,
    ) -> Result<DecodedMessagesChunk, ReportedFailure> {
        match chunk {
            SseChunk::Bytes(bytes) => {
                decode_messages_sse_chunk(bytes, boundaries).map_err(|error| ReportedFailure {
                    kind: "BadGatewayError".to_owned(),
                    message: error.to_string(),
                })
            }
            SseChunk::Mapping(fields) => {
                Ok(decode_messages_sse_value(&Json::Object(fields.clone())))
            }
        }
    }

    fn detached_chunk(&mut self, registry: &Arc<StreamRegistry>) -> Step {
        if self.drain.is_some() {
            return Step::Continue;
        }
        if let Some(drain) = registry.claim_drain(self.max_detached_stream_drains) {
            self.drain = Some(drain);
            Step::Continue
        } else {
            tracing::warn!(
                "async_sse_wrapper: detached-drain cap ({}) reached; billing {} partial chunks and aborting the upstream stream to stop provider billing",
                self.max_detached_stream_drains,
                self.collected.len()
            );
            self.bill(true);
            Step::Abort
        }
    }

    async fn complete<E>(&mut self, queue: &ClientQueue<E>)
    where
        E: Send + Sync,
    {
        if queue.client_detached.load(Ordering::SeqCst) {
            self.bill(true);
            return;
        }
        if !self.saw_terminal
            && !queue
                .enqueue(QueueItem::Chunk(incomplete_error_sse_event()))
                .await
        {
            self.bill(true);
            return;
        }
        self.bill(false);
        if !queue.enqueue(QueueItem::End).await {
            let _dispatched = self.logging.dispatch_pending_logging();
        }
    }

    async fn failed<E>(&self, queue: &ClientQueue<E>, error: E)
    where
        E: Clone + Send + Sync + 'static,
        ReportedFailure: From<E>,
    {
        let reported = ReportedFailure::from(error.clone());
        let failure = Arc::new(error);
        self.schedule_failure(reported.clone()).await;
        if !queue.client_detached.load(Ordering::SeqCst)
            && queue.enqueue(QueueItem::Failed(Arc::clone(&failure))).await
        {
            queue.wait_detached().await;
            if !queue.take_all_contains(&failure) {
                return;
            }
        }
        self.logging.fire_detached_failure_hook(&reported).await;
    }

    async fn limit_reached<E>(&self, queue: &ClientQueue<E>, failure: ReportedFailure) -> Step
    where
        E: Send + Sync,
    {
        let encoded = error_sse_event(&failure.message, &self.boundaries);
        self.schedule_failure(failure).await;
        match encoded {
            Ok(frame) => {
                let _queued = queue.enqueue(QueueItem::Chunk(frame)).await;
            }
            Err(error) => {
                tracing::warn!(
                    "async_sse_wrapper: the limit error frame could not be encoded: {error}"
                );
            }
        }
        Step::Abort
    }

    async fn schedule_failure(&self, failure: ReportedFailure) {
        self.failure_logging
            .schedule_failure_logging(StreamFailureCall {
                request_body: self.request_body.clone(),
                endpoint: EndpointType::Anthropic,
                failure,
                raw_bytes: self.collected.clone(),
            })
            .await;
    }

    fn bill(&mut self, teardown: bool) {
        let collected = take(&mut self.collected);
        let _logged = self.logging.bill_collected_chunks(collected, teardown);
    }
}

// B081's Anthropic error envelope, with the relay's own wire bounds
fn error_sse_event(message: &str, limits: &ApiLimits) -> Result<Vec<u8>, MessagesStreamSplitError> {
    let wire = WireDecodeLimits {
        max_json_bytes: limits.max_json_bytes,
        max_event_bytes: limits.max_event_bytes,
        max_items: limits.max_items,
    };
    let failure = StreamFailure::Unclassified {
        text: message.to_owned(),
    };
    mid_stream_error_sse_event(&failure, wire).map(String::into_bytes)
}
