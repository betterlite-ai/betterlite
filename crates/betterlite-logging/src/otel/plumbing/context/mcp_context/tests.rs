#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use rstest::rstest;

use crate::otel::config::destination::{DestinationFields, OtelDestination, SpanScope};
use crate::otel::plumbing::context::context_slots::{
    AnchoredSpan, McpTransportSpan, RequestTraceContext,
};
use crate::otel::plumbing::context::mcp_context::resolve_mcp_span_context;

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn span_context(trace_id: u128, span_id: u64) -> SpanContext {
    SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

fn transport(trace_id: u128, span_id: u64, recording: bool) -> McpTransportSpan {
    McpTransportSpan {
        span: AnchoredSpan {
            context: span_context(trace_id, span_id),
            http_route: Some("/mcp".to_owned()),
        },
        recording,
    }
}

fn carrier(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn the_client_trace_context_is_a_link_never_the_parent() {
    let mut trace = RequestTraceContext::default();
    trace.enter_mcp_message(None, Some(transport(7, 8, true)));
    let client = carrier(&[("traceparent", TRACEPARENT)]);
    let (context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(7));
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].span_context.trace_id().to_string(),
        "0af7651916cd43dd8448eb211c80319c"
    );
    assert_ne!(
        context.span().span_context().trace_id(),
        links[0].span_context.trace_id()
    );
}

#[test]
fn the_link_carries_the_client_tracestate() {
    let client = carrier(&[("traceparent", TRACEPARENT), ("tracestate", "vendor=value")]);
    let trace = RequestTraceContext::default();
    let (_context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert_eq!(links[0].span_context.trace_state().header(), "vendor=value");
}

#[test]
fn the_transport_span_parents_the_message_span() {
    let mut trace = RequestTraceContext::default();
    trace.enter_mcp_message(None, Some(transport(7, 8, true)));
    let (context, links) = resolve_mcp_span_context(&trace, None);
    assert_eq!(context.span().span_context().span_id(), SpanId::from(8_u64));
    assert!(links.is_empty());
}

#[test]
fn without_a_transport_the_message_span_roots_its_own_trace() {
    let trace = RequestTraceContext::default();
    let client = carrier(&[("traceparent", TRACEPARENT)]);
    let (context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert!(!context.span().span_context().is_valid());
    assert_eq!(links.len(), 1);
}

#[test]
fn the_root_anchor_parents_the_message_span_without_a_transport() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(3, 4),
        http_route: None,
    });
    let (context, _links) = resolve_mcp_span_context(&trace, None);
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(3));
}

#[test]
fn the_stored_carrier_is_used_when_the_caller_passes_none() {
    let mut trace = RequestTraceContext::default();
    trace.enter_mcp_message(Some(carrier(&[("traceparent", TRACEPARENT)])), None);
    let (_context, links) = resolve_mcp_span_context(&trace, None);
    assert_eq!(links.len(), 1);
}

#[test]
fn an_explicitly_empty_carrier_replaces_the_stored_one() {
    let mut trace = RequestTraceContext::default();
    trace.enter_mcp_message(Some(carrier(&[("traceparent", TRACEPARENT)])), None);
    let (_context, links) = resolve_mcp_span_context(&trace, Some(&BTreeMap::new()));
    assert!(links.is_empty());
}

#[rstest]
#[case::missing_traceparent("")]
#[case::malformed("00-not-a-traceparent")]
#[case::unsupported_version("ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")]
#[case::zero_trace_id("00-00000000000000000000000000000000-b7ad6b7169203331-01")]
fn a_carrier_that_does_not_extract_produces_no_link(#[case] traceparent: &str) {
    let trace = RequestTraceContext::default();
    let client = carrier(&[("traceparent", traceparent)]);
    let (_context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert!(links.is_empty());
}

#[test]
fn the_mcp_carrier_never_falls_back_to_the_ambient_span() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let trace = RequestTraceContext::default();
    let client = carrier(&[("traceparent", "00-not-a-traceparent")]);
    let (context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert!(!context.span().span_context().is_valid());
    assert!(links.is_empty());
}

#[test]
fn remote_baggage_is_never_inherited_from_the_carrier() {
    let trace = RequestTraceContext::default();
    let client = carrier(&[
        ("traceparent", TRACEPARENT),
        ("baggage", "litellm.team.id=spoofed"),
    ]);
    let (context, links) = resolve_mcp_span_context(&trace, Some(&client));
    assert_eq!(links.len(), 1);
    assert!(context.baggage().is_empty());
}

#[test]
fn a_message_transport_re_anchors_the_request_root() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: Some("/mcp".to_owned()),
    });
    trace.enter_mcp_message(None, Some(transport(7, 8, true)));
    let root = trace.root.as_ref().expect("root");
    assert_eq!(root.context.trace_id(), TraceId::from(7));
    assert_eq!(root.context.span_id(), SpanId::from(8_u64));
    assert_eq!(root.http_route.as_deref(), Some("/mcp"));
}

#[test]
fn a_finished_transport_never_re_anchors_the_request_root() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: None,
    });
    trace.enter_mcp_message(None, Some(transport(7, 8, false)));
    let root = trace.root.as_ref().expect("root");
    assert_eq!(root.context.trace_id(), TraceId::from(1));
}

#[test]
fn each_message_re_anchors_to_its_own_transport_not_the_session_start() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: None,
    });
    let first = trace.enter_mcp_message(None, Some(transport(7, 8, true)));
    let second = trace.enter_mcp_message(None, Some(transport(9, 10, true)));
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(9)
    );
    trace.exit_mcp_message(second);
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(9)
    );
    trace.exit_mcp_message(first);
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(9)
    );
}

#[test]
fn exiting_a_message_restores_the_previous_transport_and_carrier() {
    let mut trace = RequestTraceContext::default();
    let scope = trace.enter_mcp_message(Some(carrier(&[("traceparent", TRACEPARENT)])), None);
    assert!(trace.mcp_carrier.is_some());
    trace.exit_mcp_message(scope);
    assert!(trace.mcp_carrier.is_none());
    assert!(trace.mcp_transport.is_none());
}

#[test]
fn an_invalid_transport_is_stored_as_none_and_leaves_the_root_alone() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(1, 2),
        http_route: None,
    });
    let invalid = McpTransportSpan {
        span: AnchoredSpan {
            context: SpanContext::NONE,
            http_route: None,
        },
        recording: true,
    };
    trace.enter_mcp_message(None, Some(invalid));
    assert!(trace.mcp_transport.is_none());
    assert_eq!(
        trace.root.as_ref().expect("root").context.trace_id(),
        TraceId::from(1)
    );
}

#[test]
fn the_transport_is_readable_only_while_recording() {
    let mut trace = RequestTraceContext::default();
    trace.enter_mcp_message(None, Some(transport(7, 8, true)));
    assert!(trace.mcp_message_transport_span().is_some());
    trace.enter_mcp_message(None, Some(transport(9, 10, false)));
    assert!(trace.mcp_message_transport_span().is_none());
}

#[test]
fn exiting_a_message_restores_the_request_destinations() {
    let mut trace = RequestTraceContext {
        root: None,
        mcp_carrier: None,
        mcp_transport: None,
        destinations: vec![destination("arize")],
    };
    let scope = trace.enter_mcp_message(None, None);
    trace.destinations = vec![destination("langfuse")];
    trace.exit_mcp_message(scope);
    assert_eq!(trace.destinations.len(), 1);
    assert_eq!(
        trace.destinations[0].as_ref().callback_name.as_deref(),
        Some("arize")
    );
}

fn destination(callback_name: &str) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: "http://collector:4318".to_owned(),
        headers: BTreeMap::new(),
        resource_attributes: BTreeMap::new(),
        callback_name: Some(callback_name.to_owned()),
        protocol: None,
        span_scope: SpanScope::Full,
    })
}
