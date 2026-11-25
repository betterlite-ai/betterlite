use std::sync::Arc;

use betterlite_models::decisions::Json;
use tokio::task::JoinSet;

use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::worker::Admission;

use super::context::PassThroughStreamContext;
use super::failure::StreamFailure;
use super::lifecycle::StreamLifecycle;
use super::partial_usage::{PartialUsageCall, record_partial_usage_failure};
use super::ports::{StreamJobKind, StreamLoggingJob};

pub struct FailureSchedulingCall<'call> {
    pub lifecycle: &'call StreamLifecycle,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub endpoint_type: EndpointType,
    pub request_body: &'call Json,
    pub raw_bytes: &'call [Vec<u8>],
    pub failure: &'call StreamFailure,
    pub stream_context: Option<&'call PassThroughStreamContext>,
}

// LiteLLM's schedule_stream_failure_logging: keep the partial usage, then hand the failure to the dispatchers
pub async fn schedule_failure_logging(call: FailureSchedulingCall<'_>) {
    let lifecycle = call.lifecycle;
    if call.endpoint_type == EndpointType::Tinyfish
        && lifecycle
            .router
            .tinyfish
            .sse_poller_spawned(call.logging_obj.as_ref())
    {
        return;
    }
    let partial = PartialUsageCall {
        logging_obj: Arc::clone(&call.logging_obj),
        passthrough: Arc::clone(&lifecycle.passthrough),
        providers: Arc::clone(&lifecycle.router.providers),
        anthropic: Arc::clone(&lifecycle.anthropic_partial_usage),
        clock: Arc::clone(&lifecycle.clock),
        endpoint_type: call.endpoint_type,
        request_body: call.request_body.clone(),
        raw_bytes: call.raw_bytes.to_vec(),
        stream_context: call.stream_context.cloned(),
    };
    record_partial_off_reactor(partial).await;
    let work = lifecycle
        .failure_dispatch
        .dispatch_failure(Arc::clone(&call.logging_obj), call.failure);
    let job = StreamLoggingJob {
        kind: StreamJobKind::Failure,
        work,
    };
    if let Admission::Rejected(reason) = lifecycle.queue.submit(job) {
        tracing::error!(
            "Error scheduling stream failure logging: the job was dropped ({reason:?})"
        );
    }
}

async fn record_partial_off_reactor(partial: PartialUsageCall) {
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || record_partial_usage_failure(&partial));
    if let Some(Err(error)) = tasks.join_next().await {
        tracing::error!(
            "Error recovering the partial usage of a failed pass-through stream: {error}"
        );
    }
}

mod tests;
