use std::borrow::Cow;
use std::time::SystemTime;

use opentelemetry::KeyValue;
use opentelemetry::trace::{Span, SpanContext, Status};

mod tests;

// the SDK span pushes every set_attribute, so a repeated key would be exported twice; LiteLLM's span keeps the last value
pub(super) struct LastWins<'span, S> {
    span: &'span mut S,
    attributes: Vec<KeyValue>,
}

impl<'span, S> LastWins<'span, S>
where
    S: Span,
{
    pub(super) fn write<F>(span: &'span mut S, write: F)
    where
        F: FnOnce(&mut Self),
    {
        let mut buffered = Self {
            span,
            attributes: Vec::new(),
        };
        write(&mut buffered);
        for attribute in buffered.attributes.drain(..) {
            buffered.span.set_attribute(attribute);
        }
    }
}

impl<S> Span for LastWins<'_, S>
where
    S: Span,
{
    fn add_event_with_timestamp<T>(
        &mut self,
        name: T,
        timestamp: SystemTime,
        attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
        self.span
            .add_event_with_timestamp(name, timestamp, attributes);
    }

    fn span_context(&self) -> &SpanContext {
        self.span.span_context()
    }

    fn is_recording(&self) -> bool {
        self.span.is_recording()
    }

    fn set_attribute(&mut self, attribute: KeyValue) {
        match self
            .attributes
            .iter_mut()
            .find(|buffered| buffered.key == attribute.key)
        {
            Some(buffered) => buffered.value = attribute.value,
            None => self.attributes.push(attribute),
        }
    }

    fn set_status(&mut self, status: Status) {
        self.span.set_status(status);
    }

    fn update_name<T>(&mut self, new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
        self.span.update_name(new_name);
    }

    fn add_link(&mut self, span_context: SpanContext, attributes: Vec<KeyValue>) {
        self.span.add_link(span_context, attributes);
    }

    fn end_with_timestamp(&mut self, timestamp: SystemTime) {
        self.span.end_with_timestamp(timestamp);
    }
}
