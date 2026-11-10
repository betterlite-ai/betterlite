use std::collections::{BTreeMap, HashMap};

use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{SpanContext, TraceContextExt as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;

use crate::otel::plumbing::context::context_slots::RequestTraceContext;

pub const TRACEPARENT: &str = "traceparent";
pub const TRACESTATE: &str = "tracestate";

// litellm.integrations.otel.plumbing.context.extract_traceparent
#[must_use]
pub fn extract_traceparent(headers: &BTreeMap<String, String>) -> Option<Context> {
    if !headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case(TRACEPARENT))
    {
        return None;
    }
    Some(
        TraceContextPropagator::new()
            .extract_with_context(&Context::new(), &lowercase_carrier(headers)),
    )
}

// extraction starts from an empty context, as Python's propagator does, so a malformed header never inherits the ambient span
#[must_use]
pub fn extract_carrier(carrier: &BTreeMap<String, String>) -> Option<SpanContext> {
    let extracted = TraceContextPropagator::new()
        .extract_with_context(&Context::new(), &lowercase_carrier(carrier));
    let span_context = extracted.span().span_context().clone();
    span_context.is_valid().then_some(span_context)
}

// litellm.integrations.otel.plumbing.context.inject_trace_context
#[must_use]
pub fn inject_trace_context(
    trace: &RequestTraceContext,
    headers: &BTreeMap<String, String>,
    parent: Option<&SpanContext>,
) -> BTreeMap<String, String> {
    let Some(context) = outgoing_trace_context(trace, parent) else {
        return headers.clone();
    };
    let mut carrier: HashMap<String, String> = headers
        .iter()
        .filter(|(key, _)| {
            !key.eq_ignore_ascii_case(TRACEPARENT) && !key.eq_ignore_ascii_case(TRACESTATE)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let propagated = propagated_context(headers, &context);
    TraceContextPropagator::new().inject_context(&propagated, &mut carrier);
    if carrier.get(TRACESTATE).is_some_and(String::is_empty) {
        carrier.remove(TRACESTATE);
    }
    carrier.into_iter().collect()
}

// litellm.integrations.opentelemetry.OpenTelemetry.get_traceparent_from_header
#[must_use]
pub fn traceparent_from_header(
    headers: Option<&BTreeMap<String, Option<String>>>,
) -> Option<Context> {
    let provided = headers?;
    if !matches!(provided.get(TRACEPARENT), Some(Some(_))) {
        return None;
    }
    let mut carrier = HashMap::new();
    for key in [TRACEPARENT, TRACESTATE] {
        if let Some(Some(value)) = provided.get(key) {
            carrier.insert(key.to_owned(), value.clone());
        }
    }
    Some(TraceContextPropagator::new().extract_with_context(&Context::new(), &carrier))
}

fn lowercase_carrier(headers: &BTreeMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(key, value)| (key.to_lowercase(), value.clone()))
        .collect()
}

fn outgoing_trace_context(
    trace: &RequestTraceContext,
    parent: Option<&SpanContext>,
) -> Option<Context> {
    if let Some(explicit) = parent.filter(|candidate| candidate.is_valid()) {
        return Some(Context::new().with_remote_span_context(explicit.clone()));
    }
    if let Some(root) = trace
        .root
        .as_ref()
        .filter(|candidate| candidate.context.is_valid())
    {
        return Some(Context::new().with_remote_span_context(root.context.clone()));
    }
    let current = Context::current();
    if current.span().span_context().is_valid() {
        return Some(current);
    }
    None
}

// an explicit upstream traceparent of another trace is forwarded rather than swapped for ours
fn propagated_context(headers: &BTreeMap<String, String>, request: &Context) -> Context {
    let Some(caller) = extract_traceparent(headers) else {
        return request.clone();
    };
    let caller_span = caller.span().span_context().clone();
    let request_span = request.span().span_context().clone();
    if !caller_span.is_valid() || caller_span.trace_id() == request_span.trace_id() {
        return request.clone();
    }
    caller
}

#[path = "w3c_propagation/tests.rs"]
mod tests;
