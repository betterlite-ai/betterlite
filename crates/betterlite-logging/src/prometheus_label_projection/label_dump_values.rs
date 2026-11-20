use betterlite_prometheus::label_sanitization::sanitize_label_value;
use betterlite_prometheus::label_value_record::LabelDumpValue;

// Python stringifies the tags list and the custom metadata map in model_dump, but neither key is a Prometheus label name
#[must_use]
pub fn sanitized_scalar(value: &LabelDumpValue) -> Option<String> {
    match value {
        LabelDumpValue::String(text) => sanitize_label_value(Some(text.as_str())),
        LabelDumpValue::Null
        | LabelDumpValue::Boolean(_)
        | LabelDumpValue::Integer(_)
        | LabelDumpValue::Strings(_)
        | LabelDumpValue::Metadata(_) => None,
    }
}
