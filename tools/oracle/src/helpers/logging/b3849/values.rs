use std::collections::BTreeMap;

use betterlite_logging::otel::fan_out::attributes::attribute_map;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::{Array, Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::SpanData;
use serde_json::{Value as Json, json};

use crate::helpers::HelperError;

pub(super) fn attributes_of(
    values: &BTreeMap<String, OtelModelValue>,
) -> Result<Vec<KeyValue>, Box<HelperError>> {
    values
        .iter()
        .map(|(key, value)| Ok(KeyValue::new(Key::new(key.clone()), otel_value(value)?)))
        .collect()
}

fn otel_value(value: &OtelModelValue) -> Result<Value, Box<HelperError>> {
    match value {
        OtelModelValue::Bool(flag) => Ok(Value::Bool(*flag)),
        OtelModelValue::String(text) => Ok(Value::from(text.clone())),
        OtelModelValue::Number(number) => number
            .as_i64()
            .map(Value::I64)
            .or_else(|| number.as_f64().map(Value::F64))
            .ok_or_else(|| {
                Box::new(HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "unrepresentable number".to_owned(),
                })
            }),
        OtelModelValue::Array(items) => {
            let texts = items
                .iter()
                .map(text_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(Array::String(texts)))
        }
        OtelModelValue::Null(()) | OtelModelValue::Object(_) => Err(Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "unsupported span attribute value".to_owned(),
        })),
    }
}

fn text_value(item: &OtelModelValue) -> Result<StringValue, Box<HelperError>> {
    match item {
        OtelModelValue::String(text) => Ok(StringValue::from(text.clone())),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Array(_)
        | OtelModelValue::Object(_) => Err(Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "string arrays only".to_owned(),
        })),
    }
}

pub(super) fn view_json(span: &SpanData) -> Json {
    let attributes = attribute_map(&span.attributes);
    let fields = Json::Object(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), json_value(value)))
            .collect(),
    );
    json!({"name": span.name.as_ref(), "attributes": fields})
}

fn json_value(value: &Value) -> Json {
    if let Value::Array(array) = value {
        return array_json(array);
    }
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(integer) => json!(integer),
        Value::F64(float) => json!(float),
        Value::String(text) => json!(text.as_str()),
        Value::Array(_) | _ => Json::Null,
    }
}

fn array_json(array: &Array) -> Json {
    if let Array::Bool(flags) = array {
        return json!(flags);
    }
    if let Array::I64(integers) = array {
        return json!(integers);
    }
    if let Array::F64(floats) = array {
        return json!(floats);
    }
    if let Array::String(texts) = array {
        return json!(texts.iter().map(StringValue::as_str).collect::<Vec<&str>>());
    }
    Json::Null
}
