use std::collections::BTreeMap;

use opentelemetry::KeyValue;
use opentelemetry::trace::Span;

use crate::otel::legacy_policy::text_field;
use crate::otel::model::utils::coercion::{OtelModelValue, as_int};
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::vocabulary::constants::attribute_keys::{error, exception_event, http};

mod tests;

pub const ERROR_CODE: &str = "error.code";
pub const ERROR_LLM_PROVIDER: &str = "error.llm_provider";
pub const ERROR_STACK_TRACE: &str = "error.stack_trace";

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct RecordedException {
    pub type_name: String,
    pub message: String,
    pub stacktrace: String,
}

pub fn record_exception_on_span<S>(
    span: &mut S,
    exception: Option<&RecordedException>,
    payload: &BTreeMap<String, OtelModelValue>,
) where
    S: Span,
{
    if let Some(recorded) = exception {
        let mut attributes = vec![
            KeyValue::new(exception_event::TYPE, recorded.type_name.clone()),
            KeyValue::new(exception_event::MESSAGE, recorded.message.clone()),
        ];
        if !recorded.stacktrace.is_empty() {
            attributes.push(KeyValue::new(
                exception_event::STACKTRACE,
                recorded.stacktrace.clone(),
            ));
        }
        span.add_event(exception_event::NAME, attributes);
    }
    match payload.get("error_information") {
        None | Some(OtelModelValue::Null(())) => {
            write_text(span, payload, "error_str", error::MESSAGE);
        }
        Some(value) => {
            let Some(fields) = value.as_object() else {
                return;
            };
            write_error_information(span, fields);
        }
    }
}

fn write_error_information<S>(span: &mut S, fields: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    if let Some(code) = text_field(fields, "error_code") {
        let value = OtelModelValue::String(code.to_owned());
        span.safe_set_attribute(ERROR_CODE, &value);
        // LiteLLM exposes the code under the OTel name only when Python's int() reads it
        if let Some(status) = as_int(&value) {
            span.set_attribute(KeyValue::new(http::RESPONSE_STATUS_CODE, status));
        }
    }
    write_text(span, fields, "error_class", error::TYPE);
    write_text(span, fields, "error_message", error::MESSAGE);
    write_text(span, fields, "llm_provider", ERROR_LLM_PROVIDER);
    write_text(span, fields, "traceback", ERROR_STACK_TRACE);
}

fn write_text<S>(
    span: &mut S,
    fields: &BTreeMap<String, OtelModelValue>,
    key: &str,
    attribute: &str,
) where
    S: Span,
{
    if let Some(text) = text_field(fields, key) {
        span.safe_set_attribute(attribute, &OtelModelValue::String(text.to_owned()));
    }
}
