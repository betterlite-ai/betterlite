use crate::otel::model::utils::coercion::OtelModelValue;

#[must_use]
pub fn coerce_text(value: &OtelModelValue) -> Option<String> {
    match value {
        OtelModelValue::String(text) => Some(text.clone()),
        OtelModelValue::Array(parts) => {
            let collected = parts.iter().filter_map(part_text).collect::<Vec<_>>();
            if collected.is_empty() {
                None
            } else {
                Some(collected.join("\n"))
            }
        }
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Object(_) => None,
    }
}

fn part_text(part: &OtelModelValue) -> Option<String> {
    match part {
        OtelModelValue::String(text) => Some(text.clone()),
        OtelModelValue::Object(fields) => {
            let text = fields
                .get("text")
                .and_then(OtelModelValue::as_str)
                .filter(|text| !text.is_empty());
            text.or_else(|| fields.get("input_text").and_then(OtelModelValue::as_str))
                .map(str::to_owned)
        }
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Array(_) => None,
    }
}
