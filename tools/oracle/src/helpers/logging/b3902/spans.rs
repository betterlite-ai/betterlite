use std::borrow::Cow;
use std::time::UNIX_EPOCH;

use betterlite_models::decisions::Json;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Array, InstrumentationScope, KeyValue, StringValue, Value as OtelValue};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use serde::Deserialize;
use serde_json::{Number, Value, json};

use crate::helpers::HelperError;

use super::failure;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanInput {
    name: String,
    #[serde(default)]
    attributes: Vec<(String, Json)>,
}

pub(crate) fn span(input: &SpanInput) -> Result<SpanData, Box<HelperError>> {
    let mut attributes = Vec::with_capacity(input.attributes.len());
    for (key, value) in &input.attributes {
        attributes.push(KeyValue::new(key.clone(), attribute_value(value)?));
    }
    Ok(SpanData {
        span_context: SpanContext::new(
            TraceId::from(1_u128),
            SpanId::from(2_u64),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Owned(input.name.clone()),
        start_time: UNIX_EPOCH,
        end_time: UNIX_EPOCH,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("langfuse-sdk").build(),
    })
}

fn attribute_value(value: &Json) -> Result<OtelValue, Box<HelperError>> {
    match value {
        Json::Text(text) => Ok(OtelValue::from(text.clone())),
        Json::Bool(flag) => Ok(OtelValue::Bool(*flag)),
        Json::Number(number) => number_value(number),
        Json::List(items) => array_value(items),
        Json::Null | Json::Object(_) => Err(failure(
            "ValueError",
            "attribute values are text, bool, number or a homogeneous list",
        )),
    }
}

fn number_value(number: &Number) -> Result<OtelValue, Box<HelperError>> {
    if let Some(integer) = number.as_i64() {
        return Ok(OtelValue::I64(integer));
    }
    if let Some(float) = number.as_f64() {
        return Ok(OtelValue::F64(float));
    }
    Err(failure(
        "ValueError",
        "attribute numbers must fit an i64 or an f64",
    ))
}

fn array_value(items: &[Json]) -> Result<OtelValue, Box<HelperError>> {
    let booleans: Option<Vec<bool>> = items.iter().map(boolean).collect();
    if let Some(values) = booleans {
        return Ok(OtelValue::Array(Array::Bool(values)));
    }
    let integers: Option<Vec<i64>> = items.iter().map(integer).collect();
    if let Some(values) = integers {
        return Ok(OtelValue::Array(Array::I64(values)));
    }
    let floats: Option<Vec<f64>> = items.iter().map(float).collect();
    if let Some(values) = floats {
        return Ok(OtelValue::Array(Array::F64(values)));
    }
    let strings: Option<Vec<StringValue>> = items.iter().map(string).collect();
    if let Some(values) = strings {
        return Ok(OtelValue::Array(Array::String(values)));
    }
    Err(failure(
        "ValueError",
        "attribute lists must hold values of one kind",
    ))
}

const fn boolean(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(flag) => Some(*flag),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn float(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) if number.as_i64().is_none() => number.as_f64(),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

fn string(value: &Json) -> Option<StringValue> {
    match value {
        Json::Text(text) => Some(StringValue::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn span_json(span: &SpanData) -> Value {
    let attributes: Vec<Value> = span
        .attributes
        .iter()
        .map(|attribute| {
            let value = match &attribute.value {
                OtelValue::Bool(flag) => json!(flag),
                OtelValue::I64(number) => json!(number),
                OtelValue::F64(number) => {
                    Number::from_f64(*number).map_or(Value::Null, Value::Number)
                }
                OtelValue::String(text) => json!(text.as_str()),
                OtelValue::Array(array) => array_json(array),
                _ => Value::Null,
            };
            json!([attribute.key.as_str(), value])
        })
        .collect();
    json!({"name": span.name.as_ref(), "attributes": attributes})
}

fn array_json(array: &Array) -> Value {
    match array {
        Array::Bool(values) => json!(values),
        Array::I64(values) => json!(values),
        Array::F64(values) => Value::Array(
            values
                .iter()
                .map(|number| Number::from_f64(*number).map_or(Value::Null, Value::Number))
                .collect(),
        ),
        Array::String(values) => {
            Value::Array(values.iter().map(|text| json!(text.as_str())).collect())
        }
        _ => Value::Array(Vec::new()),
    }
}
