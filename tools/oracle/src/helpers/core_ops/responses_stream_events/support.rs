use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use betterlite_core::api::responses_stream_events::{
    ResponsesEventTransform, ResponsesStreamBaseState, ResponsesStreamHiddenParams,
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
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::json;
use thiserror::Error;
use tokio_util::task::TaskTracker;

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

fn generic_event(fields: IndexMap<String, Json>) -> GenericEvent {
    let r#type = match fields.get("type") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    };
    let extra = fields
        .into_iter()
        .filter(|(name, _value)| name != "type")
        .collect();
    GenericEvent { r#type, extra }
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
    ResponsesStreamBaseState {
        model: input.model.clone(),
        custom_llm_provider: input.provider.clone(),
        litellm_metadata: Some(input.metadata.clone()),
        request_data: input.request_data.clone(),
        call_type: Some("responses".to_owned()),
        finished: false,
        failure_handled: false,
        yielded_first_chunk: false,
        output_started: false,
        completed_response: None,
        completed_response_cached: false,
        completed_response_logged: false,
        completed_response_cache_hit: None,
        persist_before_logging: true,
        generated_content: String::new(),
        generated_tool_arguments: String::new(),
        start_time: probe.clock,
        created_timestamp: 0.0,
        max_streaming_duration_seconds: None,
        hidden_params: ResponsesStreamHiddenParams {
            model_id: model_id(&input.metadata),
            api_base: None,
            custom_llm_provider: input.provider.clone(),
            additional_headers: IndexMap::new(),
        },
        raw_response_headers: IndexMap::new(),
        pending_tasks: TaskTracker::new(),
    }
}

fn model_id(metadata: &IndexMap<String, Json>) -> Option<String> {
    let Some(Json::Object(info)) = metadata.get("model_info") else {
        return None;
    };
    let Some(Json::Text(id)) = info.get("id") else {
        return None;
    };
    Some(id.clone())
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
