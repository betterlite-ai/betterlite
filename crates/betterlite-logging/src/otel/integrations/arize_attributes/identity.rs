use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::keys;
use super::truthy::is_truthy;

const SESSION_SOURCES: [(&str, &str); 3] = [
    ("user_api_key_team_id", keys::LITELLM_TEAM_ID),
    ("user_api_key_team_alias", keys::LITELLM_TEAM_ALIAS),
    ("user_api_key_alias", keys::LITELLM_KEY_ALIAS),
];

pub(super) fn set_session_and_user_attrs<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    standard_logging_object: Option<&MetadataMap>,
) where
    S: Span,
{
    let Some(payload) = standard_logging_object else {
        return;
    };
    let empty = MetadataMap::new();
    let metadata = match payload.get("metadata") {
        Some(OtelModelValue::Object(fields)) => fields,
        Some(value) if is_truthy(value) => return,
        _ => &empty,
    };
    if let Some(session_id) = metadata
        .get("user_api_key_end_user_id")
        .filter(|value| is_truthy(value))
    {
        span.safe_set_attribute(keys::SESSION_ID, session_id);
    }
    if let Some(trace_id) = payload.get("trace_id").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(keys::LITELLM_TRACE_ID, trace_id);
    }
    if !has_user(kwargs, payload)
        && let Some(user_id) = metadata
            .get("user_api_key_user_id")
            .filter(|value| is_truthy(value))
    {
        span.safe_set_attribute(keys::USER_ID, user_id);
    }
    for (source, attribute) in SESSION_SOURCES {
        if let Some(value) = metadata.get(source).filter(|value| is_truthy(value)) {
            span.safe_set_attribute(attribute, value);
        }
    }
}

fn has_user(kwargs: &MetadataMap, payload: &MetadataMap) -> bool {
    let optional_user = kwargs
        .get("optional_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("user"))
        .is_some_and(is_truthy);
    let model_user = payload
        .get("model_parameters")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("user"))
        .is_some_and(is_truthy);
    optional_user || model_user
}

pub(super) fn set_response_cost_attr<S>(span: &mut S, standard_logging_object: Option<&MetadataMap>)
where
    S: Span,
{
    let Some(cost) = standard_logging_object.and_then(|payload| payload.get("response_cost"))
    else {
        return;
    };
    let Some(amount) = cost_value(cost) else {
        return;
    };
    span.safe_set_attribute(keys::LLM_COST_TOTAL, &amount);
    span.safe_set_attribute(keys::LLM_RESPONSE_COST, &amount);
}

fn cost_value(cost: &OtelModelValue) -> Option<OtelModelValue> {
    let parsed = match cost {
        OtelModelValue::Bool(flag) => {
            if *flag {
                1.0_f64
            } else {
                0.0_f64
            }
        }
        OtelModelValue::Number(number) => number.as_f64()?,
        OtelModelValue::String(text) => text.trim().parse::<f64>().ok()?,
        OtelModelValue::Null(()) | OtelModelValue::Array(_) | OtelModelValue::Object(_) => {
            return None;
        }
    };
    // a non-finite float has no serde_json number, so from_f64 refuses it
    serde_json::Number::from_f64(parsed).map(OtelModelValue::Number)
}
