use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::rules::RuleEvaluator;
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

fn raw_logging<E>(wrapper: &CallWrapper<E>, request: &RequestMap) -> Option<RawLogging> {
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
    let state = LoggingState::new(
        init,
        setup,
        wrapper.setup.redactor.as_ref(),
        next_trace_id,
        clock,
    )
    .ok()?;
    Some(RawLogging { state, correlation })
}

// LiteLLM's providers set raw_request_typed_dict to a dict; anything else falls through to the error form
fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null | Json::Number(_) => false,
        Json::Bool(value) => *value,
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn failure_text(error: &CallError) -> String {
    if error.kind == CallErrorKind::RawRequestCaptured {
        return RAW_REQUEST_CAPTURED.to_owned();
    }
    error.to_string()
}

fn error_object(message: String) -> Json {
    Json::Object(IndexMap::from([("error".to_owned(), Json::Text(message))]))
}

pub async fn return_raw_request<E>(
    wrapper: &CallWrapper<E>,
    target: &dyn CallTarget,
    mut request: RequestMap,
) -> Json
where
    E: RuleEvaluator,
{
    let Some(raw) = raw_logging(wrapper, &request) else {
        return error_object(String::from(
            "the raw request logging state could not be built",
        ));
    };
    let handle = LoggingHandle {
        state: Arc::new(Mutex::new(raw.state)),
        correlation: Arc::new(Mutex::new(raw.correlation)),
        deferred: Arc::new(Mutex::new(None)),
    };
    request.insert(
        "api_key".to_owned(),
        Json::Text(RAW_REQUEST_FAKE_KEY.to_owned()),
    );
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
    if let Some(value) = captured.filter(is_truthy) {
        return value;
    }
    error_object(failure.as_ref().map_or_else(String::new, failure_text))
}
