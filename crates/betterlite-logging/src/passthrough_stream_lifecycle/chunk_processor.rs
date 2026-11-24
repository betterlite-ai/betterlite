use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::{Stream, StreamExt};
use indexmap::IndexMap;

// ast-grep-ignore: no-long-names
use crate::passthrough_stream_logging::model::extract_model_for_cost_injection;
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::sse_frames::split_complete_sse_frames;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::worker::Admission;

use super::context::PassThroughStreamContext;
use super::cost_injection::cost_injection_active;
use super::failure::StreamFailure;
use super::failure_scheduling::{FailureSchedulingCall, schedule_failure_logging};
use super::first_token::stamp_first_chunk_if_needed;
use super::lifecycle::{StreamLifecycle, SuccessJobCall};

const RUN_ID_KEY: &[u8] = b"run_id";

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
        let ChunkProcessorCall {
            lifecycle,
            logging_obj,
            endpoint_type,
            start_time,
            url_route,
            request_body,
            status_code,
        } = call;
        let resolved_request_body = request_body.unwrap_or_else(|| Json::Object(IndexMap::new()));
        let model_from_details = logging_obj
            .read(&mut |state| match state.model_call_details.get("model") {
                Some(Json::Text(model)) => Some(model.clone()),
                Some(_) | None => None,
            })
            .flatten();
        // ast-grep-ignore: no-long-names
        let model_name = extract_model_for_cost_injection(
            Some(&resolved_request_body),
            model_from_details.as_deref(),
            &url_route,
            endpoint_type,
        );
        // Resolved once here, so the per-chunk branch below stays a plain check.
        let injection_model: Option<String> = match &model_name {
            Some(model) => {
                if cost_injection_active(
                    lifecycle.cost_injection_enabled,
                    Some(model),
                    endpoint_type,
                    &url_route,
                ) {
                    Some(model.clone())
                } else {
                    None
                }
            }
            None => None,
        };
        let mut finish = CompletionGuard {
            parts: Some(GuardParts {
                lifecycle: Arc::clone(&lifecycle),
                logging_obj: Arc::clone(&logging_obj),
                endpoint_type,
                start_time,
                url_route: url_route.clone(),
                request_body: resolved_request_body.clone(),
                status_code,
                raw_bytes: Vec::new(),
                logging_scheduled: false,
            }),
        };
        let mut tinyfish_scan_active = endpoint_type == EndpointType::Tinyfish;
        let mut tinyfish_pending: Vec<u8> = Vec::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut chunks = Box::pin(chunks);
        while let Some(item) = chunks.next().await {
            let chunk = match item {
                Ok(chunk) => chunk,
                Err(failure) => {
                    tracing::error!("Error in chunk_processor: {}", failure.message);
                    if status_code < 400 {
                        finish.mark_scheduled();
                        let context = PassThroughStreamContext {
                            success_handler: Arc::clone(&lifecycle.passthrough),
                            url_route: url_route.clone(),
                            start_time,
                        };
                        schedule_failure_logging(FailureSchedulingCall {
                            lifecycle: lifecycle.as_ref(),
                            logging_obj: Arc::clone(&logging_obj),
                            endpoint_type,
                            request_body: &resolved_request_body,
                            raw_bytes: finish.raw_bytes(),
                            failure: &failure,
                            stream_context: Some(&context),
                        })
                        .await;
                    }
                    yield Err(failure);
                    return;
                }
            };
            finish.record(&chunk);
            stamp_first_chunk_if_needed(&logging_obj, lifecycle.clock.as_ref());
            match injection_model.as_deref() {
                Some(model) => {
                    pending.extend_from_slice(&chunk);
                    let (complete, tail) = split_complete_sse_frames(&pending);
                    let complete = complete.to_vec();
                    let next_pending = tail.to_vec();
                    pending = next_pending;
                    if !complete.is_empty() {
                        let transformed =
                            lifecycle.cost_injection.inject(&complete, model, &logging_obj);
                        yield Ok(transformed);
                    }
                }
                None => {
                    if tinyfish_scan_active {
                        let mut reassembled = std::mem::take(&mut tinyfish_pending);
                        reassembled.extend_from_slice(&chunk);
                        let (complete, tail) = split_complete_sse_frames(&reassembled);
                        if contains_run_id(complete)
                            && let Some(run_id) =
                                lifecycle.router.tinyfish.run_id_from_sse_frames(complete)
                        {
                            lifecycle.router.tinyfish.start_sse_run_billing(
                                &run_id,
                                &logging_obj,
                                start_time,
                            );
                            tinyfish_scan_active = false;
                        }
                        tinyfish_pending = tail.to_vec();
                    }
                    yield Ok(chunk);
                }
            }
        }
        if !pending.is_empty() {
            yield Ok(pending);
        }
        finish.complete();
    }
}

fn contains_run_id(frames: &[u8]) -> bool {
    frames
        .windows(RUN_ID_KEY.len())
        .any(|window| window == RUN_ID_KEY)
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
}

impl CompletionGuard {
    fn record(&mut self, chunk: &[u8]) {
        if let Some(parts) = self.parts.as_mut() {
            parts.raw_bytes.push(chunk.to_vec());
        }
    }

    fn raw_bytes(&self) -> &[Vec<u8>] {
        match self.parts.as_ref() {
            Some(parts) => &parts.raw_bytes,
            None => &[],
        }
    }

    fn mark_scheduled(&mut self) {
        if let Some(parts) = self.parts.as_mut() {
            parts.logging_scheduled = true;
        }
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
            raw_bytes: std::mem::take(&mut parts.raw_bytes),
        });
        // bind_budget_reservation_to_callbacks(litellm_logging_obj.litellm_params)
        let binder = Arc::clone(&parts.lifecycle.binder);
        parts
            .logging_obj
            .edit(&mut |state| binder.bind(&mut state.litellm_params));
        deferred.park(job);
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
            raw_bytes: std::mem::take(&mut parts.raw_bytes),
        });
        if let Admission::Rejected(_reason) = parts.lifecycle.queue.submit(job) {
            tracing::error!("Error scheduling chunk_processor logging: the job was dropped");
            return;
        }
        let binder = Arc::clone(&parts.lifecycle.binder);
        parts
            .logging_obj
            .edit(&mut |state| binder.bind(&mut state.litellm_params));
    }
}
