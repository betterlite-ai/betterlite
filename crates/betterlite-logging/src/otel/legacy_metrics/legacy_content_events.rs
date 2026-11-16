use std::collections::BTreeMap;

use opentelemetry::logs::AnyValue;

use super::any_value::any_value;
use super::error::LegacyMetricsError;
use super::events::{EventAttributes, EventCall, SemanticEventEmitter, choices_of};
use super::{COMPLETION_EVENT, PROMPT_EVENT};
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::model::utils::coercion::OtelModelValue;

mod tests;

type Fields = BTreeMap<String, OtelModelValue>;

struct EventRecord {
    attributes: EventAttributes,
    body: Option<AnyValue>,
}

fn event_object<'item>(
    item: &'item OtelModelValue,
    field: &'static str,
) -> Result<&'item Fields, LegacyMetricsError> {
    item.as_object().ok_or(LegacyMetricsError::WrongType {
        field,
        expected: "an object",
    })
}

// only a string content is an attribute: LiteLLM's SDK record drops a list of parts with a warning
fn content_text(content: Option<&OtelModelValue>) -> Option<&str> {
    content
        .filter(|content| is_truthy(content))
        .and_then(OtelModelValue::as_str)
}

fn prompt_record(fields: &Fields, capture: bool, provider: Option<&str>) -> EventRecord {
    let role = fields
        .get("role")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("user");
    let mut attributes: EventAttributes = vec![("event_name", AnyValue::from(PROMPT_EVENT))];
    if let Some(name) = provider {
        attributes.push(("gen_ai.system", AnyValue::from(name.to_owned())));
    }
    let tool_id = fields.get("id").filter(|id| is_truthy(id));
    if let Some(id) = tool_id.filter(|_| role == "tool").and_then(any_value) {
        attributes.push(("id", id));
    }
    if let Some(text) = content_text(fields.get("content")).filter(|_| capture) {
        attributes.push(("gen_ai.prompt", AnyValue::from(text.to_owned())));
    }
    let mut body = fields.clone();
    if !capture {
        body.remove("content");
    }
    EventRecord {
        attributes,
        body: any_value(&OtelModelValue::Object(body)),
    }
}

fn completion_record(
    index: i64,
    fields: &Fields,
    capture: bool,
    provider: Option<&str>,
) -> EventRecord {
    let mut attributes: EventAttributes = vec![("event_name", AnyValue::from(COMPLETION_EVENT))];
    if let Some(name) = provider {
        attributes.push(("gen_ai.system", AnyValue::from(name.to_owned())));
    }
    attributes.push(("index", AnyValue::from(index)));
    if let Some(reason) = fields.get("finish_reason").and_then(any_value) {
        attributes.push(("finish_reason", reason));
    }
    let empty = Fields::new();
    let message = fields
        .get("message")
        .and_then(OtelModelValue::as_object)
        .unwrap_or(&empty);
    let role = message
        .get("role")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("assistant");
    let mut body_message =
        Fields::from([("role".to_owned(), OtelModelValue::String(role.to_owned()))]);
    let content = message.get("content").filter(|content| is_truthy(content));
    if let Some(content_value) = content.filter(|_| capture) {
        body_message.insert("content".to_owned(), content_value.clone());
        if let Some(text) = content_text(content) {
            attributes.push(("message.content", AnyValue::from(text.to_owned())));
        }
    }
    let mut body = Fields::from([
        (
            "index".to_owned(),
            OtelModelValue::Number(serde_json::Number::from(index)),
        ),
        ("message".to_owned(), OtelModelValue::Object(body_message)),
    ]);
    if let Some(reason) = fields.get("finish_reason") {
        body.insert("finish_reason".to_owned(), reason.clone());
    }
    EventRecord {
        attributes,
        body: any_value(&OtelModelValue::Object(body)),
    }
}

pub fn emit_prompt_events(
    emitter: &SemanticEventEmitter,
    call: &EventCall<'_>,
    provider: Option<&str>,
) -> Result<(), LegacyMetricsError> {
    let messages = call
        .kwargs
        .get("messages")
        .and_then(OtelModelValue::as_array)
        .unwrap_or_default();
    for message in messages {
        let fields = event_object(message, "messages")?;
        let record = prompt_record(fields, call.capture_mode.capture_in_event(), provider);
        emitter.emit_record(call.span_context, record.body, record.attributes);
    }
    Ok(())
}

pub fn emit_completion_events(
    emitter: &SemanticEventEmitter,
    call: &EventCall<'_>,
    provider: Option<&str>,
) -> Result<(), LegacyMetricsError> {
    for (index, choice) in (0_i64..).zip(choices_of(call.response)) {
        let fields = event_object(choice, "choices")?;
        let capture = call.capture_mode.capture_in_event();
        let record = completion_record(index, fields, capture, provider);
        emitter.emit_record(call.span_context, record.body, record.attributes);
    }
    Ok(())
}
