use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::constants::attribute_keys::litellm;

const FIRST_HANDOFF_KEY: &str = "first_api_call_start_time";
const RECEIVED_AT_KEY: &str = "litellm_received_at";
const DATE_TIME: &str = "%Y-%m-%d %H:%M:%S%.f";

// ast-grep-ignore: no-long-names
pub fn set_preprocessing_duration_attribute(span: Option<&LiveSpan>, container: &MetadataMap) {
    let Some(span) = span else {
        return;
    };
    let Some(first_handoff) = present(container.get(FIRST_HANDOFF_KEY)) else {
        return;
    };
    let Some(received_at) = received_at(container) else {
        return;
    };
    let (Some(start), Some(end)) = (timestamp(received_at), timestamp(first_handoff)) else {
        return;
    };
    // float operators are denied; mul_add and mul compute the same bits as Python's (end - start) * 1000.0
    let duration_ms = start.mul_add(-1.0, end).mul(1000.0);
    // clock skew -> omit rather than emit a negative latency
    if duration_ms < 0.0 {
        return;
    }
    // a NaN or infinite duration cannot be an OTel attribute, and Python's float would carry it
    let Some(number) = serde_json::Number::from_f64(duration_ms) else {
        return;
    };
    span.lock()
        .safe_set_attribute(litellm::PREPROCESSING_MS, &OtelModelValue::Number(number));
}

// litellm_received_at rides litellm_params.metadata first, then metadata, then litellm_metadata
fn received_at(container: &MetadataMap) -> Option<&OtelModelValue> {
    let params = container
        .get("litellm_params")
        .and_then(OtelModelValue::as_object);
    [
        params
            .and_then(|fields| fields.get("metadata"))
            .and_then(OtelModelValue::as_object),
        container
            .get("metadata")
            .and_then(OtelModelValue::as_object),
        container
            .get("litellm_metadata")
            .and_then(OtelModelValue::as_object),
    ]
    .into_iter()
    .flatten()
    .find_map(|fields| present(fields.get(RECEIVED_AT_KEY)))
}

fn present(value: Option<&OtelModelValue>) -> Option<&OtelModelValue> {
    value.filter(|value| !matches!(value, OtelModelValue::Null(())))
}

// _to_timestamp: epoch seconds, or a datetime string in one of the two formats strptime accepts
fn timestamp(value: &OtelModelValue) -> Option<f64> {
    match value {
        OtelModelValue::Number(number) => number.as_f64(),
        OtelModelValue::String(text) => parse_datetime(text),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Array(_)
        | OtelModelValue::Object(_) => None,
    }
}

// a naive timestamp is read as UTC; LiteLLM's datetime.strptime reads the host's local zone (docs/deviations/B3858.yaml)
fn parse_datetime(text: &str) -> Option<f64> {
    let naive = NaiveDateTime::parse_from_str(text, DATE_TIME).ok()?;
    let utc = naive.and_utc();
    let seconds = Decimal::from(utc.timestamp());
    let nanos = Decimal::new(i64::from(utc.timestamp_subsec_nanos()), 9);
    seconds.checked_add(nanos).and_then(|total| total.to_f64())
}
