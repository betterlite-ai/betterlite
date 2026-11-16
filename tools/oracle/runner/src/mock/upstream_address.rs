use serde_json::Value;

use crate::case::UpstreamAnswer;

mod tests;

const UPSTREAM_MARKER: &str = "{{upstream}}";

pub(crate) fn fill(answer: &mut UpstreamAnswer, base_url: &str) {
    if let Some(body) = answer.body.as_mut()
        && body.get("body_kind").is_none()
    {
        fill_strings(body, base_url);
    }
    if let Some(events) = answer.events.as_mut() {
        for event in events {
            *event = event.replace(UPSTREAM_MARKER, base_url);
        }
    }
}

// only string values change, so an object key or a non-string value keeps its text and type
fn fill_strings(value: &mut Value, base_url: &str) {
    match value {
        Value::String(text) => *text = text.replace(UPSTREAM_MARKER, base_url),
        Value::Array(items) => {
            for item in items {
                fill_strings(item, base_url);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                fill_strings(field, base_url);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
