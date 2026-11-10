use opentelemetry::KeyValue;
use opentelemetry::Value;
use opentelemetry::trace::{Span, Status};

use crate::otel::mappers::base::AttributeMap;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::vocabulary::constants::attribute_keys::{
    error as error_keys, exception_event, litellm_error,
};

// which of the two side effects a caller's span lifecycle already records itself
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorStamp {
    pub record_event: bool,
    pub set_status: bool,
}

impl ErrorStamp {
    pub const FULL: Self = Self {
        record_event: true,
        set_status: true,
    };
    pub const ATTRIBUTES_ONLY: Self = Self {
        record_event: false,
        set_status: false,
    };
}

// litellm.integrations.otel.emitter._resolve_error
#[must_use]
pub fn resolve_error(error: &SpanError) -> Option<(String, String)> {
    if error.error_type.is_none() && error.message.is_none() {
        return None;
    }
    let error_type = error
        .error_type
        .clone()
        .unwrap_or_else(|| "error".to_owned());
    let message = error
        .message
        .clone()
        .or_else(|| error.error_type.clone())
        .unwrap_or_else(|| "error".to_owned());
    Some((error_type, message))
}

// litellm.integrations.otel.emitter.error_attributes
#[must_use]
pub fn error_attributes(error: &SpanError) -> AttributeMap {
    let Some((error_type, message)) = resolve_error(error) else {
        return AttributeMap::new();
    };
    let mut attributes = AttributeMap::new();
    attributes.insert(error_keys::TYPE.to_owned(), Value::from(error_type));
    attributes.insert(error_keys::MESSAGE.to_owned(), Value::from(message));
    if let Some(code) = &error.code {
        attributes.insert(litellm_error::CODE.to_owned(), Value::from(code.clone()));
    }
    if let Some(stack_trace) = &error.stack_trace {
        attributes.insert(
            litellm_error::STACK_TRACE.to_owned(),
            Value::from(stack_trace.clone()),
        );
    }
    if let Some(llm_provider) = &error.llm_provider {
        attributes.insert(
            litellm_error::LLM_PROVIDER.to_owned(),
            Value::from(llm_provider.clone()),
        );
    }
    attributes
}

// litellm.integrations.otel.emitter.stamp_error
pub fn stamp_error<S>(
    span: &mut S,
    error: &SpanError,
    stamp: ErrorStamp,
) -> Option<(String, String)>
where
    S: Span,
{
    let resolved = resolve_error(error)?;
    for (key, value) in error_attributes(error) {
        span.set_attribute(KeyValue::new(key, value));
    }
    if stamp.set_status {
        span.set_status(Status::error(resolved.1.clone()));
    }
    if stamp.record_event {
        span.add_event(
            exception_event::NAME,
            vec![
                KeyValue::new(exception_event::TYPE, resolved.0.clone()),
                KeyValue::new(exception_event::MESSAGE, resolved.1.clone()),
            ],
        );
    }
    Some(resolved)
}

#[path = "error_stamping/tests.rs"]
mod tests;
