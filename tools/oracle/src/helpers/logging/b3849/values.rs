use std::collections::BTreeMap;

use betterlite_logging::otel::fan_out::attributes::attribute_map;
use betterlite_logging::otel::mappers::base::AttributeMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::trace::{Event, SpanId, Status};
use opentelemetry::{Array, Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SpanData;
use serde_json::{Value as Json, json};

use crate::helpers::HelperError;

pub(super) fn error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn attributes_of(
    values: &BTreeMap<String, OtelModelValue>,
) -> Result<Vec<KeyValue>, Box<HelperError>> {
    let mut attributes = Vec::with_capacity(values.len());
    for (key, value) in values {
        attributes.push(KeyValue::new(Key::new(key.clone()), otel_value(value)?));
    }
    Ok(attributes)
}

fn otel_value(value: &OtelModelValue) -> Result<Value, Box<HelperError>> {
    match value {
        OtelModelValue::Bool(flag) => Ok(Value::Bool(*flag)),
        OtelModelValue::String(text) => Ok(Value::from(text.clone())),
        OtelModelValue::Number(number) => number
            .as_i64()
            .map(Value::I64)
            .or_else(|| number.as_f64().map(Value::F64))
            .ok_or_else(|| error("InvalidInput", "unrepresentable number".to_owned())),
        OtelModelValue::Array(items) => string_array(items),
        OtelModelValue::Null(()) | OtelModelValue::Object(_) => Err(error(
            "InvalidInput",
            "unsupported span attribute value".to_owned(),
        )),
    }
}

fn string_array(items: &[OtelModelValue]) -> Result<Value, Box<HelperError>> {
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        match item {
            OtelModelValue::String(text) => texts.push(StringValue::from(text.clone())),
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_) => {
                return Err(error(
                    "InvalidInput",
                    "only string arrays stand for OTel attribute arrays".to_owned(),
                ));
            }
        }
    }
    Ok(Value::Array(Array::String(texts)))
}

pub(super) fn resource_of(values: &BTreeMap<String, String>) -> Resource {
    Resource::builder_empty()
        .with_attributes(
            values.iter().map(|(key, value)| {
                KeyValue::new(Key::new(key.clone()), Value::from(value.clone()))
            }),
        )
        .build()
}

pub(super) fn view_json(span: &SpanData) -> Json {
    json!({
        "name": span.name.as_ref(),
        "attributes": attributes_json(&attribute_map(&span.attributes)),
        "events": span.events.events.iter().map(event_json).collect::<Vec<Json>>(),
        "status": status_json(&span.status),
        "parent": parent_json(span.parent_span_id),
    })
}

pub(super) fn resource_json(resource: &Resource) -> Json {
    let mut fields = serde_json::Map::new();
    for (key, value) in resource {
        fields.insert(key.to_string(), json_value(value));
    }
    Json::Object(fields)
}

fn attributes_json(attributes: &AttributeMap) -> Json {
    Json::Object(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), json_value(value)))
            .collect(),
    )
}

fn event_json(event: &Event) -> Json {
    let attributes: AttributeMap = event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    json!({"name": event.name.as_ref(), "attributes": attributes_json(&attributes)})
}

fn status_json(status: &Status) -> Json {
    let (code, description) = match status {
        Status::Unset => ("unset", ""),
        Status::Ok => ("ok", ""),
        Status::Error { description } => ("error", description.as_ref()),
    };
    json!({"code": code, "description": description})
}

fn parent_json(parent: SpanId) -> Json {
    if parent == SpanId::INVALID {
        Json::Null
    } else {
        Json::String(format!("{parent:016x}"))
    }
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
