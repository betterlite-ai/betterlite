use super::common::{field, string};
use super::extraction::auth_metadata;
use super::session_id::caller_session_id;
use super::time_to_chunk::time_to_first_chunk_seconds;
use crate::otel::model::trace_controls::{TraceControls, caller_trace_controls};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::resolvers::resolve_operation;
use std::collections::BTreeMap;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LLMCallEvent {
    pub call_id: Option<String>,
    pub payload: Option<OtelModelValue>,
    pub dynamic_params: Option<OtelModelValue>,
    pub auth_metadata: Option<BTreeMap<String, String>>,
    pub is_no_upstream_call: bool,
    pub upstream_started: bool,
    pub provisional_span_name: String,
    pub time_to_first_chunk_seconds: Option<f64>,
    pub trace: TraceControls,
    pub session_id: Option<String>,
}

impl LLMCallEvent {
    #[must_use]
    pub fn from_dict(kwargs: &OtelModelValue) -> Self {
        let payload = field(kwargs, "standard_logging_object")
            .filter(|value| value.as_object().is_some())
            .cloned();
        let auth_metadata = auth_metadata(payload.as_ref(), kwargs);
        let call_id = payload
            .as_ref()
            .and_then(|value| {
                string(field(value, "litellm_call_id")).or_else(|| string(field(value, "id")))
            })
            .or_else(|| string(field(kwargs, "litellm_call_id")));
        let call_type = string(field(kwargs, "call_type"));
        let operation = resolve_operation(call_type.as_deref().unwrap_or("")).to_string();
        let model = string(field(kwargs, "model"));
        let trace = caller_trace_controls(kwargs);
        Self {
            call_id,
            payload,
            dynamic_params: field(kwargs, "standard_callback_dynamic_params").cloned(),
            auth_metadata,
            is_no_upstream_call: field(kwargs, "litellm_no_upstream_llm_call")
                .and_then(OtelModelValue::as_bool)
                .unwrap_or(false),
            upstream_started: field(kwargs, "api_call_start_time")
                .is_some_and(|value| !matches!(value, OtelModelValue::Null(()))),
            provisional_span_name: format!("{operation} {}", model.as_deref().unwrap_or(""))
                .trim()
                .to_owned(),
            time_to_first_chunk_seconds: time_to_first_chunk_seconds(kwargs),
            session_id: caller_session_id(kwargs, &trace),
            trace,
        }
    }
}

mod tests;
