use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use betterlite_core::api::responses_stream_events::{
    BaseStreamStateInput, LoggingSnapshot, ResponsesApiBase, ResponsesEventTransform,
    ResponsesStreamBaseState, StreamTimes, base_stream_state,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::api_base_lookup::{ApiBaseError, ApiBaseParams};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::json;
use thiserror::Error;

use super::Input;
use super::probe::Probe;

impl ResponsesEventTransform for Probe {
    type Error = TransformFailure;

    fn transform(
        &self,
        _model: &str,
        chunk: &Json,
        _logging: &mut IndexMap<String, Json>,
    ) -> Result<ResponsesAPIStreamingResponse, TransformFailure> {
        let Json::Object(fields) = chunk else {
            return Err(TransformFailure);
        };
        Ok(event(fields))
    }
}

#[derive(Debug, Error)]
#[error("the provider transform failed")]
pub(crate) struct TransformFailure;

fn event(fields: &IndexMap<String, Json>) -> ResponsesAPIStreamingResponse {
    match transform_streaming_response(fields) {
        DecodedStreamResponse::Known { record, .. } => ResponsesAPIStreamingResponse::Known(record),
        DecodedStreamResponse::Generic(generic) => ResponsesAPIStreamingResponse::Generic(generic),
        DecodedStreamResponse::Lax { fields, .. } => {
            ResponsesAPIStreamingResponse::Generic(generic_event(fields))
        }
    }
}

fn generic_event(mut fields: IndexMap<String, Json>) -> GenericEvent {
    let r#type = match fields.shift_remove("type") {
        Some(Json::Text(text)) => text,
        _ => String::new(),
    };
    GenericEvent {
        r#type,
        extra: fields,
    }
}

impl UsageTokenization for Probe {
    type Error = Infallible;

    fn messages(&self, request: &EstimateRequest<'_>) -> Result<Vec<Json>, Infallible> {
        self.record(json!([
            "estimate",
            request.model,
            request.request_input,
            request.responses_api_request,
            request.generated_text,
        ]));
        Ok(vec![request.request_input.clone()])
    }

    fn count(&self, request: TokenCountRequest<'_>) -> Result<i64, Infallible> {
        if request.count_response_tokens {
            return Ok(i64::try_from(request.text.map_or(0, str::len)).unwrap_or(i64::MAX));
        }
        Ok(1000 + i64::try_from(request.model.len()).unwrap_or(i64::MAX))
    }
}

pub(crate) fn state(input: &Input, probe: &Probe) -> ResponsesStreamBaseState {
    base_stream_state(&BaseStreamStateInput {
        model: &input.model,
        custom_llm_provider: input.provider.as_deref(),
        litellm_metadata: Some(&input.metadata),
        request_data: Some(&input.request_data),
        call_type: Some("responses"),
        logging: LoggingSnapshot {
            start_time: Some(probe.clock),
            litellm_params: IndexMap::new(),
        },
        headers: &IndexMap::new(),
        api_base: &NoApiBase,
        times: StreamTimes {
            now: probe.clock,
            created_timestamp: 0.0,
        },
        max_streaming_duration_seconds: None,
    })
}

struct NoApiBase;

impl ResponsesApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

pub(crate) fn logging_object(
    redactor: &Redactor,
) -> Result<StreamLoggingObject, LoggingStateError> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "responses".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
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
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )?;
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    })
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

#[derive(Debug, Error)]
pub(crate) enum LoggingStateError {
    #[error(transparent)]
    Time(#[from] chrono::ParseError),
    #[error(transparent)]
    State(#[from] betterlite_logging::logging_state::LoggingStateError),
}
