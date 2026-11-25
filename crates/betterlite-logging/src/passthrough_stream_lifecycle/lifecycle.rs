use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use tokio_util::task::TaskTracker;

use crate::logging_success::ports::SuccessClock;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::passthrough_stream_logging::build::StreamLoggingInput;
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::router::StreamLoggingRouter;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::reservation_binding::ReservationBinder;

use super::cost_injection::StreamCostInjection;
use super::failure::StreamFailureDispatch;
use super::partial_usage::AnthropicPartialUsage;
use super::ports::{
    DeferredStreamLogging, StreamJobKind, StreamLoggingJob, StreamLoggingQueue, StreamRoute,
};

// The ports one pass-through stream needs; B8931 wires them and the stream owns the values it reads
pub struct StreamLifecycle {
    pub tasks: TaskTracker,
    pub router: Arc<StreamLoggingRouter>,
    pub passthrough: Arc<PassThroughEndpointLogging>,
    pub route: Arc<dyn StreamRoute>,
    pub cost_injection: Arc<dyn StreamCostInjection>,
    pub anthropic_partial_usage: Arc<dyn AnthropicPartialUsage>,
    pub failure_dispatch: Arc<dyn StreamFailureDispatch>,
    pub queue: Arc<dyn StreamLoggingQueue>,
    pub binder: Arc<dyn ReservationBinder>,
    pub deferred: Option<Arc<dyn DeferredStreamLogging>>,
    pub clock: Arc<dyn SuccessClock>,
    pub cost_injection_enabled: bool,
}

// The values _build_logging_coroutine closes over, with end_time read when the coroutine is built
pub struct SuccessJobCall {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub url_route: String,
    pub request_body: Option<Json>,
    pub raw_bytes: Vec<Vec<u8>>,
}

impl StreamLifecycle {
    #[must_use]
    pub fn success_job(&self, call: SuccessJobCall) -> StreamLoggingJob {
        let input = StreamLoggingInput {
            logging_obj: call.logging_obj,
            passthrough: Arc::clone(&self.passthrough),
            providers: Arc::clone(&self.router.providers),
            url_route: call.url_route,
            request_body: call.request_body,
            endpoint_type: call.endpoint_type,
            start_time: call.start_time,
            end_time: self.clock.now(),
            raw_bytes: call.raw_bytes,
            model: None,
        };
        StreamLoggingJob {
            kind: StreamJobKind::Success,
            work: Arc::clone(&self.route).route(input),
        }
    }
}
