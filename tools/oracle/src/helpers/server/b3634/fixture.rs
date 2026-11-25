use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler as _, EndpointType, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::pass_through_logging::anthropic_passthrough_logging::AnthropicPassthroughLogger;
use chrono::{DateTime, FixedOffset, Utc};
use indexmap::IndexMap;

use super::Input;

pub(super) const CALL_ID: &str = "b3634-call";

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn start_time() -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
        .map(|parsed| parsed.with_timezone(&Utc))
        .or_value_error()
}

// The streaming seam's call for the case: the case's route and lines, with the case's body as the request.
pub(super) fn collected_call<'case>(
    case: &'case Input,
    logging_obj: &'case StreamLoggingObject,
    passthrough: &'case PassThroughEndpointLogging,
    request_body: &'case Json,
    start_time: DateTime<FixedOffset>,
) -> CollectedChunksCall<'case> {
    CollectedChunksCall {
        logging_obj,
        passthrough,
        url_route: &case.url_route,
        request_body: Some(request_body),
        endpoint_type: EndpointType::Anthropic,
        start_time,
        end_time: start_time,
        lines: &case.lines,
        model: None,
    }
}

// The stream seam builds its response from the case's lines, with the case's body as the request.
pub(super) fn chunks_outcome(
    logger: &AnthropicPassthroughLogger,
    case: &Input,
    logging_obj: &StreamLoggingObject,
) -> Result<HandlerOutcome, Box<HelperError>> {
    let passthrough = PassThroughEndpointLogging {
        injected_log_dispatch: None,
    };
    let request_body = Json::Object(case.request_body.clone());
    let start_time = start_time()?.fixed_offset();
    let call = collected_call(case, logging_obj, &passthrough, &request_body, start_time);
    logger.handle(&call).or_value_error()
}

// LiteLLM's Logging(model="", stream=False, litellm_call_id=CALL_ID) with the case's details written over it
pub(super) fn logging_object(
    details: &JsonObject,
) -> Result<StreamLoggingObject, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let mut logging = LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: "pass_through_endpoint".to_owned(),
            start_time: start_time()?.fixed_offset(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "b3634".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("b3634-trace".to_owned()),
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
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "b3634-trace".to_owned(),
        || Duration::ZERO,
    )
    .or_value_error()?;
    logging.model_call_details.extend(
        details
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoDispatch),
    })
}
