use super::extraction::auth_metadata;
use super::session_id::caller_session_id;
use super::time_to_chunk::time_to_first_chunk_seconds;
use super::wire::{MetadataError, callback::Callback, payload::Payload};
use crate::otel::model::trace_controls::{TraceControls, caller_trace_controls};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::resolvers::resolve_operation;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LLMCallEvent {
    pub call_id: Option<String>,
    pub payload: Option<Payload>,
    pub dynamic_params: Option<OtelModelValue>,
    pub auth_metadata: Option<BTreeMap<String, String>>,
    pub is_no_upstream_call: bool,
    pub upstream_started: bool,
    pub provisional_span_name: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub time_to_first_chunk_seconds: Option<Decimal>,
    pub trace: TraceControls,
    pub session_id: Option<String>,
}

impl LLMCallEvent {
    pub fn from_dict(kwargs: &Callback) -> Result<Self, MetadataError> {
        let payload = kwargs.standard_logging_object.as_ref();
        let operation = resolve_operation(kwargs.call_type.as_deref().unwrap_or("")).to_string();
        let raw = serde_json::from_value(serde_json::to_value(kwargs)?)?;
        let trace = caller_trace_controls(&raw);
        Ok(Self {
            call_id: payload
                .and_then(|closed| closed.litellm_call_id.clone().or_else(|| closed.id.clone()))
                .or_else(|| kwargs.litellm_call_id.clone()),
            payload: payload.cloned(),
            dynamic_params: kwargs.dynamic_params.clone(),
            auth_metadata: auth_metadata(payload, kwargs),
            is_no_upstream_call: kwargs.litellm_no_upstream_llm_call,
            upstream_started: kwargs.api_call_start_time.is_some(),
            provisional_span_name: format!("{operation} {}", kwargs.model.as_deref().unwrap_or(""))
                .trim()
                .to_owned(),
            time_to_first_chunk_seconds: time_to_first_chunk_seconds(kwargs),
            session_id: caller_session_id(kwargs, &trace),
            trace,
        })
    }
}

mod tests;
