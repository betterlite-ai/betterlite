use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_logging::otel::config::destination::{
    DestinationFields, OtelDestination, SpanScope,
};
use betterlite_logging::otel::plumbing::context::context_slots::{AnchoredSpan, McpTransportSpan};
use opentelemetry::trace::{
    Span, SpanContext, SpanId, Status, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue};

use crate::mcp::message_tracing::MessageTrace;

pub(crate) fn span_context(trace_id: u128, span_id: u64) -> SpanContext {
    SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

pub(crate) fn transport(trace_id: u128, span_id: u64, recording: bool) -> McpTransportSpan {
    McpTransportSpan {
        span: AnchoredSpan {
            context: span_context(trace_id, span_id),
            http_route: Some("/mcp".to_owned()),
        },
        recording,
    }
}

pub(crate) fn destination(callback_name: &str) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some(callback_name.to_owned()),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}

pub(crate) fn carrier(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn message(
    transport: McpTransportSpan,
    destinations: Option<Vec<OtelDestination>>,
) -> MessageTrace {
    MessageTrace {
        context: None,
        transport: Some(transport),
        destinations,
    }
}

// a span whose lifetime the test can watch through the Arc it holds, without a running SDK provider
struct TrackedSpan {
    context: SpanContext,
    _alive: Arc<()>,
}

pub(crate) fn tracked_context(trace_id: u128, span_id: u64) -> (Context, Arc<()>) {
    let alive = Arc::new(());
    let span = TrackedSpan {
        context: span_context(trace_id, span_id),
        _alive: Arc::clone(&alive),
    };
    (Context::new().with_span(span), alive)
}

impl Span for TrackedSpan {
    fn add_event<T>(&mut self, _name: T, _attributes: Vec<KeyValue>)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_event_with_timestamp<T>(
        &mut self,
        _name: T,
        _timestamp: SystemTime,
        _attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
    }

    fn span_context(&self) -> &SpanContext {
        &self.context
    }

    fn is_recording(&self) -> bool {
        true
    }

    fn set_attribute(&mut self, _attribute: KeyValue) {}

    fn set_status(&mut self, _status: Status) {}

    fn update_name<T>(&mut self, _new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_link(&mut self, _span_context: SpanContext, _attributes: Vec<KeyValue>) {}

    fn end_with_timestamp(&mut self, _timestamp: SystemTime) {}
}
