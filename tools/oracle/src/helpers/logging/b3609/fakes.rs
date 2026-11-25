use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_lifecycle::cost_injection::StreamCostInjection;
use betterlite_logging::passthrough_stream_lifecycle::failure::{
    StreamFailure, StreamFailureDispatch,
};
use betterlite_logging::passthrough_stream_lifecycle::partial_usage::AnthropicPartialUsage;
use betterlite_logging::passthrough_stream_lifecycle::ports::{
    DeferredStreamLogging, StreamLoggingJob, StreamLoggingQueue, StreamRoute, StreamWork,
};
use betterlite_logging::passthrough_stream_logging::build::StreamLoggingInput;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::router::AnthropicFrameProbe;
use betterlite_logging::passthrough_stream_logging::tinyfish::{
    TinyfishCollectedChunks, TinyfishStreamBilling,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
use betterlite_logging::reservation_binding::{ReservationBinder, stamp_callback_bound};
use betterlite_logging::worker::Admission;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::json;

pub(super) struct Handler;

impl CollectedChunksHandler for Handler {
    fn handle(&self, _call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let body: Json = serde_json::from_value(json!({
            "id": "chatcmpl-1",
            "usage": {"prompt_tokens": 10_u32, "completion_tokens": 5_u32, "total_tokens": 15_u32},
        }))
        .map_err(HandlerError::leaf)?;
        let mut kwargs = PassThroughLoggingKwargs::default();
        let number = "0.25"
            .parse::<serde_json::Number>()
            .map_err(HandlerError::leaf)?;
        kwargs
            .entries
            .insert("response_cost".to_owned(), Json::Number(number));
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(body)),
            kwargs,
        })
    }
}

pub(super) struct PassThrough;

impl AnthropicFrameProbe for PassThrough {
    fn is_message_stop(&self, _frames: &[u8]) -> bool {
        false
    }

    fn is_provider_error(&self, _frames: &[u8]) -> bool {
        false
    }
}

impl AnthropicPartialUsage for PassThrough {
    fn record_partial_usage(
        &self,
        _logging_obj: &StreamLoggingObject,
        _request_body: &Json,
        _all_chunks: &[Vec<u8>],
    ) {
    }
}

#[async_trait]
impl StreamSuccessDispatch for PassThrough {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) struct FixedClock(pub(super) DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

pub(super) struct Recorder {
    poller: bool,
    run_id: Option<String>,
    pub(super) routes: Mutex<Vec<Vec<Vec<u8>>>>,
    pub(super) queued: Mutex<Vec<StreamLoggingJob>>,
    pub(super) parked: Mutex<Vec<StreamLoggingJob>>,
    pub(super) failures: Mutex<Vec<StreamFailure>>,
    pub(super) scans: Mutex<usize>,
    pub(super) started: Mutex<Vec<(String, DateTime<FixedOffset>)>>,
}

impl Recorder {
    pub(super) const fn new(poller: bool, run_id: Option<String>) -> Self {
        Self {
            poller,
            run_id,
            routes: Mutex::new(Vec::new()),
            queued: Mutex::new(Vec::new()),
            parked: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            scans: Mutex::new(0),
            started: Mutex::new(Vec::new()),
        }
    }
}

impl StreamRoute for Recorder {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
        Box::pin(async move {
            self.routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(input.raw_bytes);
        })
    }
}

impl StreamLoggingQueue for Recorder {
    fn submit(&self, job: StreamLoggingJob) -> Admission {
        self.queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
        Admission::Accepted
    }
}

impl DeferredStreamLogging for Recorder {
    fn park(&self, job: StreamLoggingJob) {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
    }
}

impl StreamCostInjection for Recorder {
    fn inject(
        &self,
        frames: &[u8],
        _model_name: &str,
        _logging_obj: &StreamLoggingObject,
    ) -> Vec<u8> {
        let mut output = b"injected:".to_vec();
        output.extend_from_slice(frames);
        output
    }
}

impl ReservationBinder for Recorder {
    fn bind(&self, litellm_params: &mut IndexMap<String, Json>) {
        stamp_callback_bound(litellm_params, true);
    }
}

impl StreamFailureDispatch for Recorder {
    fn dispatch_failure(
        &self,
        _logging_obj: Arc<StreamLoggingObject>,
        failure: &StreamFailure,
    ) -> StreamWork {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(failure.clone());
        Box::pin(async {})
    }
}

#[async_trait]
impl TinyfishStreamBilling for Recorder {
    // The helper input represents a poller already active before the stream fails.
    fn sse_poller_spawned(&self, _logging_obj: &StreamLoggingObject) -> bool {
        self.poller
    }

    fn run_id_from_sse_frames(&self, _frames: &[u8]) -> Option<String> {
        let mut scan_count = self.scans.lock().unwrap_or_else(PoisonError::into_inner);
        *scan_count = scan_count.saturating_add(1);
        drop(scan_count);
        self.run_id.clone()
    }

    fn start_sse_run_billing(
        &self,
        run_id: &str,
        _logging_obj: &StreamLoggingObject,
        start_time: DateTime<FixedOffset>,
    ) {
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((run_id.to_owned(), start_time));
    }

    // ast-grep-ignore: no-long-names
    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        _call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError> {
        Ok(HandlerOutcome::default())
    }
}
