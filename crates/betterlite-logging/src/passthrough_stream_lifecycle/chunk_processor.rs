use std::mem::take;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::{Stream, StreamExt as _};
use indexmap::IndexMap;

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
        let mut run = CompletionGuard::new(call);
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

struct CompletionGuard {
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
    logging_scheduled: bool,
    injection_model: Option<String>,
    pending: Vec<u8>,
    scan_active: bool,
    scan_pending: Vec<u8>,
}

impl CompletionGuard {
    fn new(call: ChunkProcessorCall) -> Self {
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
                logging_scheduled: false,
                injection_model,
                pending: Vec::new(),
                scan_active,
                scan_pending: Vec::new(),
            }),
        }
    }

    fn record(&mut self, chunk: &[u8]) {
        if let Some(parts) = self.parts.as_mut() {
            parts.raw_bytes.push(chunk.to_vec());
        }
    }

    fn accept(&mut self, chunk: Vec<u8>) -> Option<Vec<u8>> {
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

    async fn read_failure(&mut self, failure: &StreamFailure) {
        tracing::error!("Error in chunk_processor: {}", failure.message());
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
        let failure = failure.clone();
        let task = tokio::spawn(async move {
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
        let _ = task.await;
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
        let (release, wait) = tokio::sync::oneshot::channel();
        let mut job = job;
        let work = std::mem::replace(&mut job.work, Box::pin(async {}));
        job.work = Box::pin(async move {
            if wait.await.is_ok() {
                work.await;
            }
        });
        let admission = parts.lifecycle.queue.submit(job);
        if let Admission::Rejected(reason) = admission {
            tracing::error!("Error scheduling chunk_processor logging: {reason:?}");
            return;
        }
        let binder = Arc::clone(&parts.lifecycle.binder);
        parts
            .logging_obj
            .edit(&mut |state| binder.bind(&mut state.litellm_params));
        let _ = release.send(());
    }
}
