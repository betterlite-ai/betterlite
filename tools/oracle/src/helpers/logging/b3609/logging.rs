use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use indexmap::IndexMap;

use super::fakes::PassThrough;
use super::invalid;

pub(super) fn logging_object() -> Result<Arc<StreamLoggingObject>, Box<HelperError>> {
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let redactor = Redactor::new(16).map_err(|error| invalid(&error.to_string()))?;
    let mut logging = LoggingState::new(
        logging_init()?,
        setup,
        &redactor,
        || "trace-1".to_owned(),
        || Duration::ZERO,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    logging.litellm_params = reservation_params();
    Ok(Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(PassThrough),
    }))
}

fn logging_init() -> Result<LoggingInit, Box<HelperError>> {
    Ok(LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .map_err(|error| invalid(&error.to_string()))?,
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(true),
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
    })
}

fn reservation_params() -> IndexMap<String, Json> {
    let mut reservation = IndexMap::new();
    reservation.insert("id".to_owned(), Json::Text("reservation-1".to_owned()));
    let mut metadata = IndexMap::new();
    metadata.insert(
        "user_api_key_budget_reservation".to_owned(),
        Json::Object(reservation),
    );
    let mut params = IndexMap::new();
    params.insert("metadata".to_owned(), Json::Object(metadata));
    params
}
