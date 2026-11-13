use crate::datadog::apm::{NullTracer, Span, Tracer};
use crate::datadog::payloads::legacy::legacy_v1_logging_to_datadog;
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::payloads::types::DatadogPayload;

use super::support::{default_environment, default_kwargs, json, legacy_event, standard_payload};

struct TestSpan {
    trace_id: Option<u128>,
    span_id: Option<u64>,
}

impl Span for TestSpan {
    fn trace_id(&self) -> Option<u128> {
        self.trace_id
    }

    fn span_id(&self) -> Option<u64> {
        self.span_id
    }
}

struct TestTracer<'span>(Option<&'span dyn Span>, Option<&'span dyn Span>);

impl Tracer for TestTracer<'_> {
    fn current_span(&self) -> Option<&dyn Span> {
        self.0
    }

    fn current_root_span(&self) -> Option<&dyn Span> {
        self.1
    }
}

fn standard(tracer: &dyn Tracer) -> DatadogPayload {
    standard_logging_to_datadog(
        &standard_payload(),
        DataDogStatus::Info,
        &default_environment(),
        tracer,
    )
    .unwrap()
}

#[test]
fn datadog_trace_context_attachment_behavior() {
    let span = TestSpan {
        trace_id: Some(12),
        span_id: Some(34),
    };
    let current = TestTracer(Some(&span), None);
    let built = standard(&current);
    assert_eq!(built.trace_id.as_deref(), Some("12"));
    assert_eq!(built.span_id.as_deref(), Some("34"));
    let kwargs = default_kwargs();
    let response = json(r#"{"usage": {}}"#);
    let legacy = legacy_v1_logging_to_datadog(
        &legacy_event(&kwargs, &response),
        &default_environment(),
        &current,
    )
    .unwrap();
    assert_eq!(legacy.trace_id.as_deref(), Some("12"));
    assert_eq!(legacy.span_id.as_deref(), Some("34"));
    let root = TestTracer(None, Some(&span));
    assert_eq!(standard(&root).trace_id.as_deref(), Some("12"));
    let trace_only = TestSpan {
        trace_id: Some(7),
        span_id: None,
    };
    let partial = standard(&TestTracer(Some(&trace_only), None));
    assert_eq!(partial.trace_id.as_deref(), Some("7"));
    assert_eq!(partial.span_id, None);
    let id_only = TestSpan {
        trace_id: None,
        span_id: Some(9),
    };
    let skipped = standard(&TestTracer(Some(&id_only), None));
    assert_eq!(skipped.trace_id, None);
    assert_eq!(skipped.span_id, None);
    let untraced = standard(&NullTracer);
    assert_eq!(untraced.trace_id, None);
    assert_eq!(untraced.span_id, None);
}
