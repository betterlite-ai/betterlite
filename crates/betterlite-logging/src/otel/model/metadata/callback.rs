use super::common::{field, object, seconds, string};
use crate::otel::model::trace_controls::{TraceControls, caller_trace_controls};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::resolvers::resolve_operation;
use rust_decimal::prelude::ToPrimitive as _;
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

#[must_use]
pub fn caller_session_id(kwargs: &OtelModelValue, trace: &TraceControls) -> Option<String> {
    let params = object(field(kwargs, "litellm_params"));
    let bodies: Vec<&BTreeMap<String, OtelModelValue>> = ["metadata", "litellm_metadata"]
        .into_iter()
        .filter_map(|key| {
            let values = params?;
            object(values.get(key))
        })
        .collect();
    let minted: Vec<String> = bodies
        .iter()
        .filter_map(|body| {
            let generated = body.get("litellm_session_id_generated");
            generated.filter(|marker| marker.as_bool() == Some(true))?;
            string(body.get("session_id"))
        })
        .collect();
    if !minted.is_empty() {
        return trace
            .session_id
            .clone()
            .filter(|session| !minted.contains(session))
            .or_else(|| {
                bodies
                    .iter()
                    .filter_map(|body| string(body.get("session_id")))
                    .find(|session| !minted.contains(session))
            });
    }
    let explicit = params.and_then(|values| string(values.get("litellm_session_id")));
    let echoes_trace = explicit.as_ref().is_some_and(|session| {
        bodies
            .iter()
            .any(|body| string(body.get("trace_id")).as_ref() == Some(session))
    });
    explicit
        .filter(|_| !echoes_trace)
        .or_else(|| trace.session_id.clone())
}

#[must_use]
pub fn time_to_first_chunk_seconds(kwargs: &OtelModelValue) -> Option<f64> {
    let streaming = field(kwargs, "optional_params")
        .and_then(|value| field(value, "stream"))
        .and_then(OtelModelValue::as_bool)
        .unwrap_or(false);
    if !streaming {
        return None;
    }
    let elapsed = seconds(field(kwargs, "completion_start_time"))
        .zip(seconds(field(kwargs, "api_call_start_time")))
        .and_then(|(completion_start, api_call_start)| {
            completion_start.checked_sub(api_call_start)
        })?;
    elapsed.to_f64()
}

#[must_use]
pub fn auth_metadata(
    payload: Option<&OtelModelValue>,
    kwargs: &OtelModelValue,
) -> Option<BTreeMap<String, String>> {
    let payload_metadata =
        payload.and_then(|payload_value| object(field(payload_value, "metadata")));
    let params = object(field(kwargs, "litellm_params"));
    [
        payload_metadata,
        params.and_then(|values| object(values.get("metadata"))),
        params.and_then(|values| object(values.get("litellm_metadata"))),
    ]
    .into_iter()
    .filter_map(|metadata| {
        let values = metadata?;
        values.get("user_api_key_auth_metadata")
    })
    .find_map(|auth_config| {
        let values = object(Some(auth_config))?;
        let typed: BTreeMap<String, String> = values
            .iter()
            .filter_map(|(key, item)| item.as_str().map(|text| (key.clone(), text.to_owned())))
            .collect();
        (!typed.is_empty()).then_some(typed)
    })
}
