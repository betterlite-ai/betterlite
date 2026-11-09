use std::time::SystemTime;

use opentelemetry::Context;
use opentelemetry::trace::{Link, SpanContext, TraceContextExt as _};

// what the caller knows about a service span's timing; post_response is the request's phase marker
#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceSpanTiming {
    pub end_time: Option<SystemTime>,
    pub parent_end_time: Option<SystemTime>,
    pub post_response: bool,
}

// litellm.integrations.otel.plumbing.context.resolve_parent_context
#[must_use]
pub fn resolve_parent_context(threaded: Option<&SpanContext>) -> Context {
    let current = Context::current();
    let ambient = current.span().span_context().clone();
    if ambient.is_valid() {
        return current;
    }
    threaded
        .filter(|candidate| candidate.is_valid())
        .map_or_else(
            || current.clone(),
            |parent| current.with_remote_span_context(parent.clone()),
        )
}

// litellm.integrations.otel.plumbing.context.resolve_service_span_context
#[must_use]
pub fn resolve_service_span_context(
    threaded: Option<&SpanContext>,
    timing: &ServiceSpanTiming,
) -> (Context, Vec<Link>) {
    let context = resolve_parent_context(threaded);
    let parent = context.span().span_context().clone();
    if !is_post_response(timing, parent.is_valid()) {
        return (context, Vec::new());
    }
    let rooted = context.with_remote_span_context(SpanContext::NONE);
    (rooted, vec![Link::with_context(parent)])
}

// litellm.integrations.otel.plumbing.context._is_post_response
fn is_post_response(timing: &ServiceSpanTiming, parent_valid: bool) -> bool {
    if !parent_valid {
        return false;
    }
    if timing.post_response {
        return true;
    }
    timing
        .parent_end_time
        .is_some_and(|parent_end| timing.end_time.is_none_or(|end| end > parent_end))
}

#[path = "post_response/tests.rs"]
mod tests;
