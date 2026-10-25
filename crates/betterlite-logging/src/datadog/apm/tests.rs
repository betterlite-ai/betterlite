#![cfg(test)]

use std::sync::Mutex;

use super::{NullTracer, Span, TraceContext, Tracer, tag_call_id, tag_request};

#[derive(Default)]
struct RecordingSpan {
    tags: Mutex<Vec<String>>,
}

impl Span for RecordingSpan {
    fn set_tag(&self, key: &str, value: &str) -> bool {
        self.tags.lock().unwrap().push(format!("{key}={value}"));
        true
    }

    fn trace_id(&self) -> Option<u128> {
        Some(12)
    }

    fn span_id(&self) -> Option<u64> {
        Some(34)
    }
}

struct TracedSpan;

impl Span for TracedSpan {
    fn trace_id(&self) -> Option<u128> {
        Some(u128::MAX)
    }
}

struct BareSpan;

impl Span for BareSpan {}

struct TestTracer<'span>(Option<&'span dyn Span>, Option<&'span dyn Span>);

impl Tracer for TestTracer<'_> {
    fn current_span(&self) -> Option<&dyn Span> {
        self.0
    }

    fn current_root_span(&self) -> Option<&dyn Span> {
        self.1
    }
}

fn ids(tracer: &dyn Tracer) -> Option<(String, Option<String>)> {
    TraceContext::active(tracer).map(|context| (context.trace_id, context.span_id))
}

#[test]
fn datadog_span_context_behavior() {
    let span = RecordingSpan::default();
    let full = Some((String::from("12"), Some(String::from("34"))));
    assert_eq!(ids(&TestTracer(Some(&span), Some(&TracedSpan))), full);
    assert_eq!(ids(&TestTracer(None, Some(&span))), full);
    let traced = Some((u128::MAX.to_string(), None));
    assert_eq!(ids(&TestTracer(Some(&TracedSpan), None)), traced);
    assert_eq!(ids(&TestTracer(Some(&BareSpan), Some(&span))), None);
    assert_eq!(ids(&NullTracer), None);
}

#[test]
fn datadog_proxy_span_tagging_behavior() {
    let span = RecordingSpan::default();
    assert!(!TestTracer(Some(&span), None).tag_active_span("", "value"));
    assert!(!TestTracer(Some(&BareSpan), Some(&span)).tag_active_span("key", "value"));
    assert!(!NullTracer.tag_active_span("key", "value"));
    let tracer = TestTracer(None, Some(&span));
    tag_call_id(&tracer, Some("call"));
    tag_call_id(&tracer, Some(""));
    tag_request(&tracer, Some("alias"), Some(""), Some("model"));
    tag_request(&tracer, None, Some("hash"), None);
    let expected = "litellm.call_id=call litellm.key_alias=alias litellm.requested_model=model";
    let tagged = span.tags.into_inner().unwrap().join(" ");
    assert_eq!(tagged, format!("{expected} litellm.key_hash=hash"));
}
