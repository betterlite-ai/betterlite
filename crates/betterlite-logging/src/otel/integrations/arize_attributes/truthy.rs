use crate::otel::model::utils::coercion::OtelModelValue;

#[must_use]
pub(super) fn is_worth_writing(value: &OtelModelValue) -> bool {
    match value {
        OtelModelValue::Null(()) => false,
        OtelModelValue::Bool(flag) => *flag,
        OtelModelValue::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        OtelModelValue::String(text) => !text.is_empty(),
        OtelModelValue::Array(items) => !items.is_empty(),
        OtelModelValue::Object(fields) => !fields.is_empty(),
    }
}
