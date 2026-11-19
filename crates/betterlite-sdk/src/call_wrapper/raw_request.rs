use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::CallWrapper;
use super::error::{CallError, CallErrorKind};
use super::state::LoggingHandle;
use super::target::CallTarget;
use crate::call_setup::RequestMap;

pub const RAW_REQUEST_MODEL: &str = "gpt-3.5-turbo";
pub const RAW_REQUEST_CALL_ID: &str = "1234";
pub const RAW_REQUEST_FAKE_KEY: &str = "my-fake-api-key";
pub const RAW_REQUEST_CAPTURED: &str = "raw request was not captured before the provider call; check the proxy logs for the pre_call error";

struct RawLogging {
    state: LoggingState,
    correlation: CorrelationContext,
}

pub(super) fn raw_logging<E>(wrapper: &CallWrapper<E>, request: &RequestMap) -> Option<RawLogging> {
    let mut correlation = CorrelationContext::default();
    let next_trace_id = || wrapper.ids.next_call_id();
    let mut clock = || wrapper.clock.monotonic();
    let init = LoggingInit {
        model: RAW_REQUEST_MODEL.to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: wrapper.clock.now(),
        litellm_call_id: RAW_REQUEST_CALL_ID.to_owned(),
        function_id: RAW_REQUEST_CALL_ID.to_owned(),
        messages: Some(Messages::Values(vec![Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("hi".to_owned())),
        ]))])),
        stream: Some(false),
        litellm_trace_id: None,
        kwargs: request.clone(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags {
            log_raw_request_response: true,
            raw_request_only: true,
        },
        callback_policy: CallbackPolicy::default(),
    };
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let state =
        LoggingState::new(init, setup, &wrapper.setup.redactor, next_trace_id, clock).ok()?;
    Some(RawLogging { state, correlation })
}

fn failure_text(error: &CallError) -> String {
    if error.kind == CallErrorKind::RawRequestCaptured {
        return RAW_REQUEST_CAPTURED.to_owned();
    }
    error.to_string()
}

pub(super) async fn capture(
    logging: Option<RawLogging>,
    target: &dyn CallTarget,
    request: RequestMap,
) -> Json {
    let Some(raw) = logging else {
        return Json::Object(IndexMap::from([(
            "error".to_owned(),
            Json::Text(String::from(
                "the raw request logging state could not be built",
            )),
        )]));
    };
    let handle = LoggingHandle {
        state: Arc::new(Mutex::new(raw.state)),
        correlation: Arc::new(Mutex::new(raw.correlation)),
        deferred: Arc::new(Mutex::new(None)),
    };
    let failure = match target.call(request, Some(&handle)).await {
        Ok(_outcome) => None,
        Err(error) => Some(error),
    };
    let captured = {
        let state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .model_call_details
            .get("raw_request_typed_dict")
            .cloned()
    };
    // the logging state only ever holds the dict form here; any other type is not a captured request
    if let Some(value @ Json::Object(_)) = captured {
        return value;
    }
    let message = failure.as_ref().map_or_else(String::new, failure_text);
    Json::Object(IndexMap::from([("error".to_owned(), Json::Text(message))]))
}
