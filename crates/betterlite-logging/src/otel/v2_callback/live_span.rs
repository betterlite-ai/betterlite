use std::sync::{Arc, Mutex, MutexGuard};

use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, SpanContext, SpanId};
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::otel::emitter::error_stamping::{ErrorStamp, stamp_error};
use crate::otel::model::payloads::error::SpanError;

// a live SDK span every phase, identity stamp and failure hook can reach by clone
#[derive(Clone, Debug)]
pub struct LiveSpan {
    pub context: SpanContext,
    span: Arc<Mutex<SdkSpan>>,
}

impl From<SdkSpan> for LiveSpan {
    fn from(span: SdkSpan) -> Self {
        Self {
            context: span.span_context().clone(),
            span: Arc::new(Mutex::new(span)),
        }
    }
}

impl LiveSpan {
    #[must_use]
    pub fn is_recording(&self) -> bool {
        match self.span.lock() {
            Ok(guard) => guard.is_recording(),
            Err(poisoned) => poisoned.into_inner().is_recording(),
        }
    }

    #[must_use]
    pub fn has_attribute(&self, key: &str) -> bool {
        let span = self.lock();
        match span.exported_data() {
            Some(data) => data
                .attributes
                .iter()
                .any(|attribute| attribute.key.as_str() == key),
            None => false,
        }
    }

    pub fn set_attribute(&self, attribute: KeyValue) {
        match self.span.lock() {
            Ok(mut guard) => guard.set_attribute(attribute),
            Err(poisoned) => poisoned.into_inner().set_attribute(attribute),
        }
    }

    pub fn stamp(&self, error: &SpanError, stamp: ErrorStamp) {
        match self.span.lock() {
            Ok(mut guard) => {
                let _stamped = stamp_error(&mut *guard, error, stamp);
            }
            Err(poisoned) => {
                let _stamped = stamp_error(&mut *poisoned.into_inner(), error, stamp);
            }
        }
    }

    pub fn end(&self) {
        match self.span.lock() {
            Ok(mut guard) => guard.end(),
            Err(poisoned) => poisoned.into_inner().end(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SdkSpan> {
        match self.span.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

// the live handles of the server spans mark anchored, pruned to recording spans on every registration
#[derive(Debug, Default)]
pub struct RootSpans {
    spans: Mutex<Vec<(SpanId, LiveSpan)>>,
}

impl RootSpans {
    pub fn register(&self, span: &LiveSpan) {
        let id = span.context.span_id();
        let mut spans = self.lock();
        spans.retain(|(candidate, live)| *candidate != id && live.is_recording());
        spans.push((id, span.clone()));
    }

    // a handle is only good while its span records, as LiteLLM's request_root_span checks on every read
    #[must_use]
    pub fn get(&self, id: SpanId) -> Option<LiveSpan> {
        let spans = self.lock();
        let found = spans
            .iter()
            .find(|(candidate, live)| *candidate == id && live.is_recording())?;
        Some(found.1.clone())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(SpanId, LiveSpan)>> {
        match self.spans.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
