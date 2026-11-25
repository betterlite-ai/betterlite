use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;

use super::{VertexBatchInput, VertexBatchLogged, VertexBatchLogging};
use crate::pass_through_logging::anthropic_batch::ResponseIds;

pub const RESPONSE_ID: &str = "resp-1";

pub struct FixedResponseId;

impl ResponseIds for FixedResponseId {
    fn new_response_id(&self) -> String {
        RESPONSE_ID.to_owned()
    }
}

#[must_use]
pub fn logging_state() -> Option<LoggingState> {
    let init = LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::from_timestamp(0, 0)?.fixed_offset(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
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
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).ok()?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated".to_owned(),
        || Duration::ZERO,
    )
    .ok()
}

#[must_use]
pub fn handle_batch_response(body: &[u8]) -> Option<VertexBatchLogged> {
    let mut call_log = logging_state()?;
    let logging = VertexBatchLogging::new(None, None, Arc::new(FixedResponseId));
    let request_metadata = JsonObject::new();
    let input = VertexBatchInput {
        status_code: 200,
        body,
        url_route: "/vertex_ai/v1/projects/p1/locations/us-central1/batchPredictionJobs",
        start_time: DateTime::from_timestamp(0, 0)?,
        request_metadata: &request_metadata,
    };
    Some(logging.batch_prediction_jobs_handler(input, &mut call_log))
}
