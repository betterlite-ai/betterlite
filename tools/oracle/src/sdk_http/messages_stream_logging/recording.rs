use std::io;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use reqwest::Response;

use crate::sdk_http::error::SdkHttpError;

// Each reading is one second after the one before, so the construction reading is the Unix epoch
#[derive(Default)]
pub(super) struct StepClock {
    reads: AtomicI64,
}

impl SuccessClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let seconds = self.reads.fetch_add(1, Ordering::SeqCst);
        DateTime::from_timestamp_nanos(seconds.saturating_mul(1_000_000_000)).fixed_offset()
    }
}

pub(super) fn drain<T>(recorded: &Mutex<Vec<T>>) -> Vec<T> {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<u8>>>);

impl Warnings {
    pub(super) fn take(&self) -> Vec<String> {
        let text = String::from_utf8_lossy(&drain(&self.0)).into_owned();
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

impl io::Write for Warnings {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object() -> Result<Arc<StreamLoggingObject>, SdkHttpError> {
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let logging = LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: DateTime::UNIX_EPOCH.fixed_offset(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(SdkHttpError::LoggingState)?;
    Ok(Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    }))
}

pub(super) fn value_error(message: &str) -> ReportedFailure {
    ReportedFailure {
        kind: "ValueError".to_owned(),
        message: message.to_owned(),
    }
}

pub(super) fn counted_future(ran: &Arc<AtomicUsize>) -> LoggingFuture {
    let ran = Arc::clone(ran);
    Box::pin(async move {
        ran.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) routed: Mutex<Vec<RouteStreamingLogging>>,
    pub(super) processed: Mutex<Vec<ChunkProcessorCall<u16>>>,
    pub(super) route_failure: Mutex<Option<ReportedFailure>>,
    pub(super) ran: Arc<AtomicUsize>,
}

impl StreamSeams for Recorder {
    type Response = Response;
    type Stream = Response;

    fn route_streaming_logging(
        &self,
        call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        let failure = self
            .route_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(failure) = failure {
            return Err(failure);
        }
        self.routed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        Ok(counted_future(&self.ran))
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<Response>) -> Response {
        self.processed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ChunkProcessorCall {
                response: call.response.status().as_u16(),
                request_body: call.request_body,
                logging_obj: call.logging_obj,
                endpoint_type: call.endpoint_type,
                start_time: call.start_time,
                success_handler: call.success_handler,
                url_route: call.url_route,
            });
        call.response
    }
}
