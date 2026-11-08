use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::trace::Span;
use opentelemetry::{KeyValue, Value};

#[must_use]
pub fn primitive_value(value: &OtelModelValue) -> Value {
    match value {
        OtelModelValue::Null(()) => Value::from(""),
        OtelModelValue::String(text) => Value::from(text.clone()),
        OtelModelValue::Bool(flag) => Value::Bool(*flag),
        OtelModelValue::Number(number) => match number.as_i64() {
            Some(integer) => Value::I64(integer),
            None => match number.as_f64() {
                Some(float) => Value::F64(float),
                None => Value::from(""),
            },
        },
        // LiteLLM stringifies containers, and Python's repr text is outside the JSON contract
        OtelModelValue::Array(_) | OtelModelValue::Object(_) => {
            serde_json::to_string(value).map_or_else(|_| Value::from(""), Value::from)
        }
    }
}

pub trait SafeSetAttribute {
    fn safe_set_attribute(&mut self, key: &str, value: &OtelModelValue);
}

impl<S: Span> SafeSetAttribute for S {
    fn safe_set_attribute(&mut self, key: &str, value: &OtelModelValue) {
        self.set_attribute(KeyValue::new(key.to_owned(), primitive_value(value)));
    }
}

pub trait LLMObsAttributes {
    fn set_messages<S: Span>(_span: &mut S, _kwargs: &MetadataMap) {}

    fn set_response_output_messages<S: Span>(_span: &mut S, _response: &OtelModelValue) {}
}

#[path = "span_attributes/tests.rs"]
mod tests;
