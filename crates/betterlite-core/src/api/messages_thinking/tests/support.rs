#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
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
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use crate::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};

pub(super) fn time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(super) struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        time()
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn logging_object() -> Arc<StreamLoggingObject> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: time(),
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
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds");
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    })
}

#[derive(Debug, Clone)]
pub(super) struct Processed {
    pub(super) response: String,
    pub(super) request_body: IndexMap<String, Json>,
    pub(super) same_logging_object: bool,
    pub(super) url_route: String,
    pub(super) endpoint_anthropic: bool,
    pub(super) start_time: DateTime<FixedOffset>,
}

// Records B2436's chunk-processor call and answers it as the production seam would
pub(super) struct Recorder {
    pub(super) processed: Mutex<Option<Processed>>,
    pub(super) supplied: Arc<StreamLoggingObject>,
}

impl StreamSeams for Recorder {
    type Response = String;
    type Stream = String;

    fn route_streaming_logging(
        &self,
        _call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        Ok(Box::pin(async { Ok(()) }))
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<String>) -> String {
        let relayed = format!("processed {}", call.response);
        let processed = Processed {
            response: call.response,
            request_body: call.request_body,
            same_logging_object: Arc::ptr_eq(&call.logging_obj, &self.supplied),
            url_route: call.url_route.to_owned(),
            endpoint_anthropic: call.endpoint_type == EndpointType::Anthropic,
            start_time: call.start_time,
        };
        *self
            .processed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(processed);
        relayed
    }
}
