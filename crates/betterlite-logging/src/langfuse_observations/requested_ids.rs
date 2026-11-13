use std::cell::Cell;

use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator};

thread_local! {
    static REQUESTED_TRACE_ID: Cell<Option<TraceId>> = const { Cell::new(None) };
    static REQUESTED_SPAN_ID: Cell<Option<SpanId>> = const { Cell::new(None) };
}

// litellm's ContextVar pair becomes a scoped thread-local, so ids never touch OTel's own current context
#[derive(Clone, Debug, Default)]
pub struct RequestedIdGenerator {
    fallback: RandomIdGenerator,
}

impl IdGenerator for RequestedIdGenerator {
    fn new_trace_id(&self) -> TraceId {
        requested_ids()
            .0
            .unwrap_or_else(|| self.fallback.new_trace_id())
    }

    fn new_span_id(&self) -> SpanId {
        requested_ids()
            .1
            .unwrap_or_else(|| self.fallback.new_span_id())
    }
}

// litellm.integrations.langfuse.langfuse_sdk._requested_trace_id and _requested_span_id in one guard
pub struct RequestedIds {
    previous_trace_id: Option<TraceId>,
    previous_span_id: Option<SpanId>,
}

impl RequestedIds {
    #[must_use]
    pub fn enter(trace_id: Option<TraceId>, span_id: Option<SpanId>) -> Self {
        let previous = Self {
            previous_trace_id: REQUESTED_TRACE_ID.with(Cell::get),
            previous_span_id: REQUESTED_SPAN_ID.with(Cell::get),
        };
        REQUESTED_TRACE_ID.with(|slot| slot.set(trace_id));
        REQUESTED_SPAN_ID.with(|slot| slot.set(span_id));
        previous
    }
}

impl Drop for RequestedIds {
    fn drop(&mut self) {
        REQUESTED_TRACE_ID.with(|slot| slot.set(self.previous_trace_id));
        REQUESTED_SPAN_ID.with(|slot| slot.set(self.previous_span_id));
    }
}

#[must_use]
pub fn requested_ids() -> (Option<TraceId>, Option<SpanId>) {
    (
        REQUESTED_TRACE_ID.with(Cell::get),
        REQUESTED_SPAN_ID.with(Cell::get),
    )
}

#[path = "requested_ids/tests.rs"]
mod tests;
