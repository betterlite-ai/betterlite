use crate::otel::model::utils::coercion::OtelModelValue;

pub(super) fn scalar_text(value: &OtelModelValue) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    match value {
        OtelModelValue::Bool(true) => Some("True".to_owned()),
        OtelModelValue::Bool(false) => Some("False".to_owned()),
        OtelModelValue::Number(number) => Some(number.to_string()),
        OtelModelValue::Null(())
        | OtelModelValue::String(_)
        | OtelModelValue::Array(_)
        | OtelModelValue::Object(_) => None,
    }
}
