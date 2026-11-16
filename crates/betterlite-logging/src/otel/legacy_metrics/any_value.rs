use opentelemetry::logs::AnyValue;
use opentelemetry::{Key, StringValue};

use crate::otel::model::utils::coercion::OtelModelValue;

// OTLP has no null, so a null map entry or list item is left out of the body
#[must_use]
pub fn any_value(value: &OtelModelValue) -> Option<AnyValue> {
    match value {
        OtelModelValue::Null(()) => None,
        OtelModelValue::Bool(flag) => Some(AnyValue::Boolean(*flag)),
        OtelModelValue::Number(number) => number
            .as_i64()
            .map(AnyValue::Int)
            .or_else(|| number.as_f64().map(AnyValue::Double)),
        OtelModelValue::String(text) => Some(AnyValue::String(StringValue::from(text.clone()))),
        OtelModelValue::Array(items) => Some(AnyValue::ListAny(Box::new(
            items.iter().filter_map(any_value).collect(),
        ))),
        OtelModelValue::Object(fields) => Some(AnyValue::Map(Box::new(
            fields
                .iter()
                .filter_map(|(name, item)| {
                    any_value(item).map(|converted| (Key::from(name.clone()), converted))
                })
                .collect(),
        ))),
    }
}
