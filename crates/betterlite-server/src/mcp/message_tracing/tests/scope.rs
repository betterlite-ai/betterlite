use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, RequestTraceContext,
};
use betterlite_logging::otel::plumbing::context::mcp_context::resolve_mcp_span_context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    Span as _, SpanId, TraceContextExt as _, TraceId, Tracer as _, TracerProvider as _,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tokio::time::timeout;

use super::support::{carrier, destination, message, span_context, tracked_context, transport};
use crate::mcp::message_tracing::{MessageTrace, MessageTraceScope};

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

#[test]
fn absent_snapshot() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: Some(carrier(&[("traceparent", TRACEPARENT)])),
        mcp_transport: Some(transport(7, 8, true)),
        destinations: vec![destination("arize")],
    };
    let scope = MessageTraceScope::enter(&mut trace, None, None);
    assert!(scope.as_ref().mcp_carrier.is_none());
    assert_eq!(scope.as_ref().mcp_transport, Some(transport(7, 8, true)));
    assert_eq!(scope.as_ref().destinations, vec![destination("arize")]);
    drop(scope);
    assert!(trace.mcp_carrier.is_some());
    assert_eq!(trace.mcp_transport, Some(transport(7, 8, true)));
    assert_eq!(trace.destinations, vec![destination("arize")]);
}

#[test]
fn absent_request_context() {
    let snapshot = MessageTrace::capture(None, Some("/mcp"), Some(vec![destination("arize")]));
    assert!(snapshot.context.is_none());
    assert!(snapshot.transport.is_none());
    assert_eq!(snapshot.destinations, Some(vec![destination("arize")]));
}

#[test]
fn a_context_without_a_span_captures_no_transport() {
    let context = Context::new();
    let snapshot = MessageTrace::capture(Some(&context), Some("/mcp"), None);
    assert!(snapshot.transport.is_none());
    assert!(snapshot.context.is_some());
}

#[test]
fn a_non_recording_context_captures_a_finished_transport() {
    let context = Context::new().with_remote_span_context(span_context(7, 8));
    let snapshot = MessageTrace::capture(Some(&context), None, None);
    let message_transport = snapshot.transport.as_ref().expect("transport");
    assert_eq!(message_transport.span.context, span_context(7, 8));
    assert!(!message_transport.recording);
}

#[test]
fn absent_transport_preserves() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: Some(transport(7, 8, true)),
        destinations: vec![destination("arize")],
    };
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: Some("/session".to_owned()),
    });
    let snapshot = MessageTrace {
        context: None,
        transport: None,
        destinations: Some(vec![destination("langfuse")]),
    };
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    assert_eq!(scope.as_ref().mcp_transport, Some(transport(7, 8, true)));
    assert!(scope.as_ref().mcp_message_transport_span().is_some());
    assert_eq!(scope.as_ref().destinations, vec![destination("langfuse")]);
    let anchored = scope.as_ref().root.as_ref().expect("root");
    assert_eq!(anchored.context.trace_id(), TraceId::from(1));
    assert_eq!(anchored.http_route.as_deref(), Some("/session"));
    drop(scope);
    assert_eq!(trace.mcp_transport, Some(transport(7, 8, true)));
    assert_eq!(trace.destinations, vec![destination("arize")]);
    let kept = trace.root.as_ref().expect("root");
    assert_eq!(kept.context.trace_id(), TraceId::from(1));
    assert_eq!(kept.context.span_id(), SpanId::from(2));
}

#[test]
fn empty_destinations_clear() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: None,
        destinations: vec![destination("arize")],
    };
    let snapshot = message(transport(7, 8, true), Some(Vec::new()));
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    assert!(scope.as_ref().destinations.is_empty());
    drop(scope);
    assert_eq!(trace.destinations, vec![destination("arize")]);
}

#[test]
fn absent_destinations_preserve() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: None,
        destinations: vec![destination("arize")],
    };
    let snapshot = message(transport(7, 8, true), None);
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    assert_eq!(scope.as_ref().destinations, vec![destination("arize")]);
    drop(scope);
    assert_eq!(trace.destinations, vec![destination("arize")]);
}

#[test]
fn per_message_parent() {
    let mut first = RequestTraceContext::default();
    let mut second = RequestTraceContext::default();
    let first_snapshot = message(transport(7, 8, true), Some(vec![destination("arize")]));
    let second_snapshot = message(transport(9, 10, true), Some(vec![destination("langfuse")]));
    let first_scope = MessageTraceScope::enter(&mut first, None, Some(&first_snapshot));
    let second_scope = MessageTraceScope::enter(&mut second, None, Some(&second_snapshot));
    let (first_parent, first_links) = resolve_mcp_span_context(first_scope.as_ref(), None);
    let (second_parent, second_links) = resolve_mcp_span_context(second_scope.as_ref(), None);
    assert_eq!(
        first_parent.span().span_context().trace_id(),
        TraceId::from(7)
    );
    assert_eq!(
        first_parent.span().span_context().span_id(),
        SpanId::from(8)
    );
    assert_eq!(
        second_parent.span().span_context().trace_id(),
        TraceId::from(9)
    );
    assert_eq!(
        second_parent.span().span_context().span_id(),
        SpanId::from(10)
    );
    assert!(first_links.is_empty());
    assert!(second_links.is_empty());
    assert_eq!(
        first_scope.as_ref().destinations,
        vec![destination("arize")]
    );
    assert_eq!(
        second_scope.as_ref().destinations,
        vec![destination("langfuse")]
    );
    drop(first_scope);
    drop(second_scope);
    assert!(first.mcp_transport.is_none());
    assert!(second.mcp_transport.is_none());
    assert!(first.destinations.is_empty());
    assert!(second.destinations.is_empty());
}

#[test]
fn remote_context_is_link() {
    let mut trace = RequestTraceContext::default();
    let snapshot = message(transport(7, 8, true), None);
    let scope = MessageTraceScope::enter(
        &mut trace,
        Some(carrier(&[("traceparent", TRACEPARENT)])),
        Some(&snapshot),
    );
    let (parent, links) = resolve_mcp_span_context(scope.as_ref(), None);
    assert_eq!(parent.span().span_context().trace_id(), TraceId::from(7));
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].span_context.trace_id().to_string(),
        "0af7651916cd43dd8448eb211c80319c"
    );
    assert_ne!(
        parent.span().span_context().trace_id(),
        links[0].span_context.trace_id()
    );
}

#[test]
fn baggage_is_ignored() {
    let mut trace = RequestTraceContext::default();
    let snapshot = message(transport(7, 8, true), None);
    let scope = MessageTraceScope::enter(
        &mut trace,
        Some(carrier(&[
            ("traceparent", TRACEPARENT),
            ("baggage", "litellm.team.id=spoofed"),
            ("litellm.team.id", "spoofed"),
        ])),
        Some(&snapshot),
    );
    let (parent, links) = resolve_mcp_span_context(scope.as_ref(), None);
    assert_eq!(links.len(), 1);
    assert!(parent.baggage().is_empty());
}

#[test]
fn restore_on_error() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: Some(carrier(&[("traceparent", TRACEPARENT)])),
        mcp_transport: Some(transport(3, 4, true)),
        destinations: vec![destination("arize")],
    };
    let snapshot = message(transport(7, 8, true), Some(vec![destination("langfuse")]));
    let outcome: Result<(), &'static str> = {
        let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
        assert_eq!(scope.as_ref().mcp_transport, Some(transport(7, 8, true)));
        assert_eq!(scope.as_ref().destinations, vec![destination("langfuse")]);
        Err("the operation failed")
    };
    assert!(outcome.is_err());
    assert_eq!(trace.mcp_transport, Some(transport(3, 4, true)));
    assert_eq!(trace.destinations, vec![destination("arize")]);
    assert!(trace.mcp_carrier.is_some());
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(7)
    );
}

#[tokio::test]
async fn restore_on_cancel() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: Some(transport(3, 4, true)),
        destinations: vec![destination("arize")],
    };
    let snapshot = message(transport(7, 8, true), Some(vec![destination("langfuse")]));
    let outcome = timeout(Duration::ZERO, install_and_wait(&mut trace, &snapshot)).await;
    assert!(outcome.is_err());
    assert_eq!(trace.mcp_transport, Some(transport(3, 4, true)));
    assert_eq!(trace.destinations, vec![destination("arize")]);
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(7)
    );
}

async fn install_and_wait(trace: &mut RequestTraceContext, snapshot: &MessageTrace) {
    let _scope = MessageTraceScope::enter(trace, None, Some(snapshot));
    pending::<()>().await;
}

#[test]
fn root_stays_reanchored() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: Some("/mcp".to_owned()),
    });
    let snapshot = message(transport(7, 8, true), None);
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    let anchored = scope.as_ref().root.as_ref().expect("root");
    assert_eq!(anchored.context.trace_id(), TraceId::from(7));
    assert_eq!(anchored.http_route.as_deref(), Some("/mcp"));
    drop(scope);
    let kept = trace.root.as_ref().expect("root");
    assert_eq!(kept.context.trace_id(), TraceId::from(7));
    assert_eq!(kept.context.span_id(), SpanId::from(8));
}

#[test]
fn finished_transport_does_not_reanchor() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: None,
    });
    let snapshot = message(transport(7, 8, false), None);
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    assert_eq!(
        scope
            .as_ref()
            .root
            .as_ref()
            .expect("root")
            .context
            .trace_id(),
        TraceId::from(1)
    );
    assert!(scope.as_ref().mcp_message_transport_span().is_none());
    drop(scope);
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(1)
    );
}

#[test]
fn live_span_lifetime() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("message-tracing");
    let span = tracer.start("POST /mcp");
    let started = span.span_context().clone();
    let context = Context::new().with_span(span);
    let snapshot = MessageTrace::capture(Some(&context), Some("/mcp"), None);
    let message_transport = snapshot.transport.as_ref().expect("transport");
    assert_eq!(message_transport.span.context, started);
    assert_eq!(message_transport.span.http_route.as_deref(), Some("/mcp"));
    assert!(message_transport.recording);
    drop(context);
    let mut trace = RequestTraceContext::default();
    let scope = MessageTraceScope::enter(&mut trace, None, Some(&snapshot));
    assert_eq!(scope.as_ref().mcp_message_transport_span(), Some(&started));
    let live = snapshot.context.as_ref().expect("live context");
    live.span()
        .set_attribute(KeyValue::new("error.type", "tool_failure"));
    live.span().end();
    drop(scope);
    let finished = exporter.get_finished_spans().expect("finished spans");
    assert_eq!(finished.len(), 1);
    assert_eq!(
        finished[0].attributes,
        vec![KeyValue::new("error.type", "tool_failure")]
    );
}

#[test]
fn two_messages_keep_their_own_snapshots() {
    let (first_context, first_alive) = tracked_context(7, 8);
    let (second_context, second_alive) = tracked_context(9, 10);
    let first_snapshot =
        MessageTrace::capture(Some(&first_context), None, Some(vec![destination("arize")]));
    let second_snapshot = MessageTrace::capture(
        Some(&second_context),
        None,
        Some(vec![destination("langfuse")]),
    );
    drop(first_context);
    drop(second_context);
    assert_eq!(Arc::strong_count(&first_alive), 2);
    assert_eq!(Arc::strong_count(&second_alive), 2);
    let mut first_trace = RequestTraceContext::default();
    let mut second_trace = RequestTraceContext::default();
    let first_scope = MessageTraceScope::enter(&mut first_trace, None, Some(&first_snapshot));
    let second_scope = MessageTraceScope::enter(&mut second_trace, None, Some(&second_snapshot));
    assert_scope_parent(&first_scope, 7, "arize");
    assert_scope_parent(&second_scope, 9, "langfuse");
    assert_eq!(Arc::strong_count(&first_alive), 2);
    assert_eq!(Arc::strong_count(&second_alive), 2);
    drop(first_scope);
    drop(second_scope);
    assert!(first_trace.mcp_transport.is_none());
    assert!(second_trace.mcp_transport.is_none());
    assert!(first_trace.destinations.is_empty());
    assert!(second_trace.destinations.is_empty());
    drop(first_snapshot);
    assert_eq!(Arc::strong_count(&first_alive), 1);
    assert_eq!(Arc::strong_count(&second_alive), 2);
    drop(second_snapshot);
    assert_eq!(Arc::strong_count(&second_alive), 1);
}

fn assert_scope_parent(scope: &MessageTraceScope<'_>, trace_id: u128, callback_name: &str) {
    let (parent, links) = resolve_mcp_span_context(scope.as_ref(), None);
    assert!(links.is_empty());
    assert_eq!(
        parent.span().span_context().trace_id(),
        TraceId::from(trace_id)
    );
    assert_eq!(
        scope.as_ref().destinations,
        vec![destination(callback_name)]
    );
}
