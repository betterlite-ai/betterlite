use betterlite_logging::otel::mappers::base::AttributeMap;
use betterlite_logging::otel::model::metadata::wire::MetadataError;
use opentelemetry::{Array, StringValue, Value as OtelValue};
use serde_json::{Value, json};

pub(super) fn attributes_record(attributes: &AttributeMap) -> Value {
    Value::Object(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), json_value(value)))
            .collect(),
    )
}

pub(super) fn metadata_error(error: &MetadataError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "MetadataError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn invalid_input(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn json_value(value: &OtelValue) -> Value {
    match value {
        OtelValue::Bool(flag) => json!(flag),
        OtelValue::I64(integer) => json!(integer),
        OtelValue::F64(float) => json!(float),
        OtelValue::String(text) => json_attribute_text(text.as_str()),
        OtelValue::Array(array) => array_record(array),
        _ => Value::Null,
    }
}

fn array_record(array: &Array) -> Value {
    match array {
        Array::Bool(flags) => json!(flags),
        Array::I64(integers) => json!(integers),
        Array::F64(floats) => json!(floats),
        Array::String(texts) => json!(texts.iter().map(StringValue::as_str).collect::<Vec<_>>()),
        _ => Value::Null,
    }
}

fn json_attribute_text(text: &str) -> Value {
    match serde_json::from_str(text) {
        Ok(parsed) => parsed,
        Err(_failure) => json!(text),
    }
}
