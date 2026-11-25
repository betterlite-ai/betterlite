use std::mem::replace;
use std::mem::take;
use std::sync::Arc;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::{Stream, StreamExt as _};
use indexmap::IndexMap;
use tokio::sync::oneshot;

use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::worker::Admission;

use self::frames::{inject_frames, resolve_injection_model, scan_tinyfish};

use super::context::PassThroughStreamContext;
use super::failure::StreamFailure;
use super::failure_scheduling::{FailureSchedulingCall, schedule_failure_logging};
use super::first_token::stamp_first_chunk_if_needed;
use super::lifecycle::{StreamLifecycle, SuccessJobCall};

mod frames;

// LiteLLM keeps every upstream chunk; the log copy is about twice the largest stream a working setup sends
pub(super) const MAX_LOGGED_STREAM_BYTES: usize = 0x0400_0000;
// the pooled transport's reply cap, the same ceiling B1315 puts on an unterminated fragment
pub(super) const MAX_PENDING_FRAME_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

// LiteLLM grows all three stream buffers without limit; each cap stops one of them growing with the stream
#[derive(Clone, Copy)]
pub(super) struct StreamBufferCaps {
    pub(super) logged_bytes: usize,
    pub(super) frame_bytes: usize,
}

impl StreamBufferCaps {
    pub(super) const DEFAULT: Self = Self {
        logged_bytes: MAX_LOGGED_STREAM_BYTES,
        frame_bytes: MAX_PENDING_FRAME_BYTES,
    };
}

// LiteLLM's chunk_processor arguments plus the status its completion and failure gates read
pub struct ChunkProcessorCall {
    pub lifecycle: Arc<StreamLifecycle>,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub url_route: String,
    pub request_body: Option<Json>,
    pub status_code: u16,
}

// LiteLLM's chunk_processor: forward the upstream bytes, buffer them for logging, own the terminal accounting
pub fn chunk_processor<S>(
    call: ChunkProcessorCall,
    chunks: S,
) -> impl Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static
where
    S: Stream<Item = Result<Vec<u8>, StreamFailure>> + Send + 'static,
{
    async_stream::stream! {
        let mut run = CompletionGuard::new(call, StreamBufferCaps::DEFAULT);
        let mut stream = Box::pin(chunks);
        while let Some(item) = stream.next().await {
            let chunk = match item {
                Ok(chunk) => chunk,
                Err(failure) => {
                    run.read_failure(&failure).await;
                    yield Err(failure);
                    return;
                }
            };
            if let Some(output) = run.accept(chunk) {
                yield Ok(output);
            }
        }
        if let Some(tail) = run.take_tail() {
            yield Ok(tail);
        }
        run.complete();
    }
}

pub(super) struct CompletionGuard {
    parts: Option<GuardParts>,
}

struct GuardParts {
    lifecycle: Arc<StreamLifecycle>,
    logging_obj: Arc<StreamLoggingObject>,
    endpoint_type: EndpointType,
    start_time: DateTime<FixedOffset>,
    url_route: String,
    request_body: Json,
    status_code: u16,
    raw_bytes: Vec<Vec<u8>>,
    logged_bytes: usize,
    // None once the copy stopped at the cap, so the warning and the stop happen once
    logged_limit: Option<usize>,
    logging_scheduled: bool,
    injection_model: Option<String>,
    pending: Vec<u8>,
    max_frame_bytes: usize,
    scan_active: bool,
    scan_pending: Vec<u8>,
}

impl CompletionGuard {
    pub(super) fn new(call: ChunkProcessorCall, caps: StreamBufferCaps) -> Self {
        let resolved_request_body = call
            .request_body
            .unwrap_or_else(|| Json::Object(IndexMap::new()));
        let injection_model = resolve_injection_model(
            &call.lifecycle,
            &call.logging_obj,
            call.endpoint_type,
            &call.url_route,
            &resolved_request_body,
        );
        let scan_active = call.endpoint_type == EndpointType::Tinyfish;
        Self {
            parts: Some(GuardParts {
                lifecycle: call.lifecycle,
                logging_obj: call.logging_obj,
                endpoint_type: call.endpoint_type,
                start_time: call.start_time,
                url_route: call.url_route,
                request_body: resolved_request_body,
                status_code: call.status_code,
                raw_bytes: Vec::new(),
                logged_bytes: 0,
                logged_limit: Some(caps.logged_bytes),
                logging_scheduled: false,
                injection_model,
                pending: Vec::new(),
                max_frame_bytes: caps.frame_bytes,
                scan_active,
                scan_pending: Vec::new(),
            }),
        }
    }

    fn record(&mut self, chunk: &[u8]) {
        let Some(parts) = self.parts.as_mut() else {
            return;
        };
        let Some(limit) = parts.logged_limit else {
            return;
        };
        let kept = parts.logged_bytes.saturating_add(chunk.len());
        if kept > limit {
            parts.logged_limit = None;
            tracing::warn!(
                "The pass-through stream is past {limit} bytes, so its log copy stops here and the rest is forwarded without being logged"
            );
            return;
        }
        parts.logged_bytes = kept;
        parts.raw_bytes.push(chunk.to_vec());
    }

    pub(super) fn accept(&mut self, chunk: Vec<u8>) -> Option<Vec<u8>> {
        self.record(&chunk);
        let parts = self.parts.as_mut()?;
        stamp_first_chunk_if_needed(&parts.logging_obj, parts.lifecycle.clock.as_ref());
        if parts.injection_model.is_some() {
            return inject_frames(parts, &chunk);
        }
        scan_tinyfish(parts, &chunk);
        Some(chunk)
    }

    fn take_tail(&mut self) -> Option<Vec<u8>> {
        let parts = self.parts.as_mut()?;
        if parts.pending.is_empty() {
            return None;
        }
        Some(take(&mut parts.pending))
    }

    // The deferred branch of LiteLLM's try block: park the coroutine instead of enqueueing it now.
    fn complete(&mut self) {
        let Some(parts) = self.parts.as_mut() else {
            return;
        };
        if parts.logging_scheduled || parts.raw_bytes.is_empty() || parts.status_code >= 400 {
            return;
        }
        let Some(deferred) = parts.lifecycle.deferred.as_ref().map(Arc::clone) else {
            return;
        };
        parts.logging_scheduled = true;
        let job = parts.lifecycle.success_job(SuccessJobCall {
            logging_obj: Arc::clone(&parts.logging_obj),
            endpoint_type: parts.endpoint_type,
            start_time: parts.start_time,
            url_route: parts.url_route.clone(),
            request_body: Some(parts.request_body.clone()),
            raw_bytes: take(&mut parts.raw_bytes),
        });
        // bind_budget_reservation_to_callbacks(litellm_logging_obj.litellm_params)
        let binder = Arc::clone(&parts.lifecycle.binder);
        parts
            .logging_obj
            .edit(&mut |state| binder.bind(&mut state.litellm_params));
        deferred.park(job);
    }

    async fn read_failure(&mut self, stream_failure: &StreamFailure) {
        tracing::error!("Error in chunk_processor: {}", stream_failure.message());
        let Some(parts) = self.parts.as_mut() else {
            return;
        };
        if parts.status_code >= 400 {
            return;
        }
        let context = PassThroughStreamContext {
            success_handler: Arc::clone(&parts.lifecycle.passthrough),
            url_route: parts.url_route.clone(),
            start_time: parts.start_time,
        };
        let lifecycle = Arc::clone(&parts.lifecycle);
        let logging_obj = Arc::clone(&parts.logging_obj);
        let endpoint_type = parts.endpoint_type;
        let request_body = parts.request_body.clone();
        let raw_bytes = parts.raw_bytes.clone();
        let failure = stream_failure.clone();
        let task = parts.lifecycle.tasks.spawn(async move {
            schedule_failure_logging(FailureSchedulingCall {
                lifecycle: lifecycle.as_ref(),
                logging_obj,
                endpoint_type,
                request_body: &request_body,
                raw_bytes: &raw_bytes,
                failure: &failure,
                stream_context: Some(&context),
            })
            .await;
        });
        parts.logging_scheduled = true;
        if let Err(error) = task.await {
            tracing::error!("Error recovering a failed pass-through stream: {error}");
        }
    }
}

// LiteLLM's finally block: a clean end, a disconnect (GeneratorExit) or an error all pass through here once.
impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let Some(mut parts) = self.parts.take() else {
            return;
        };
        if parts.logging_scheduled || parts.raw_bytes.is_empty() || parts.status_code >= 400 {
            return;
        }
        parts.logging_scheduled = true;
        let job = parts.lifecycle.success_job(SuccessJobCall {
            logging_obj: Arc::clone(&parts.logging_obj),
            endpoint_type: parts.endpoint_type,
            start_time: parts.start_time,
            url_route: parts.url_route.clone(),
            request_body: Some(parts.request_body.clone()),
            raw_bytes: take(&mut parts.raw_bytes),
        });
        let (release, wait) = oneshot::channel();
        let mut queued_job = job;
        let work = replace(&mut queued_job.work, Box::pin(async {}));
        queued_job.work = Box::pin(async move {
            if wait.await.is_ok() {
                work.await;
            }
        });
        let admission = parts.lifecycle.queue.submit(queued_job);
        if let Admission::Rejected(reason) = admission {
            tracing::error!("Error scheduling chunk_processor logging: {reason:?}");
            return;
        }
        let binder = Arc::clone(&parts.lifecycle.binder);
        parts
            .logging_obj
            .edit(&mut |state| binder.bind(&mut state.litellm_params));
        if release.send(()).is_err() {
            tracing::error!("The queued stream logging job stopped before reservation binding");
        }
    }
}
