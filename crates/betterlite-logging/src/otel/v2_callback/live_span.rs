use std::sync::{Arc, Mutex, MutexGuard};

use opentelemetry::trace::{Span as _, SpanContext, SpanId};
use opentelemetry_sdk::trace::Span as SdkSpan;

// a live SDK span every phase, identity stamp and failure hook can reach by clone
#[derive(Clone, Debug)]
pub struct LiveSpan {
    pub context: SpanContext,
    pub span: Arc<Mutex<SdkSpan>>,
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
    pub fn has_attribute(&self, key: &str) -> bool {
        self.lock().exported_data().is_some_and(|data| {
            data.attributes
                .iter()
                .any(|attribute| attribute.key.as_str() == key)
        })
    }

    // callers hold the guard for one read, stamp or end; a poisoned lock still answers
    pub fn lock(&self) -> MutexGuard<'_, SdkSpan> {
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
    // the retain's prune of an ended same-id handle is unobservable through get, which filters on recording
    #[cfg_attr(test, mutants::skip)]
    pub fn register(&self, span: &LiveSpan) {
        let id = span.context.span_id();
        let mut spans = self.lock();
        spans.retain(|(candidate, live)| *candidate != id && live.lock().is_recording());
        spans.push((id, span.clone()));
    }

    // the recording prune is betterlite's own, so a stamp falls through to a live span (docs/deviations/B3853.yaml)
    #[must_use]
    pub fn get(&self, id: SpanId) -> Option<LiveSpan> {
        self.lock()
            .iter()
            .find(|(candidate, live)| *candidate == id && live.lock().is_recording())
            .map(|(_, live)| live.clone())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(SpanId, LiveSpan)>> {
        match self.spans.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
