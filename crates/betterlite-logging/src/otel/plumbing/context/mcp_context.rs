use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::trace::{Link, TraceContextExt as _};

use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::context::w3c_propagation::extract_carrier;

// litellm.integrations.otel.plumbing.context.resolve_mcp_span_context
#[must_use]
pub fn resolve_mcp_span_context(
    trace: &RequestTraceContext,
    carrier: Option<&BTreeMap<String, String>>,
) -> (Context, Vec<Link>) {
    let source = carrier.or(trace.mcp_carrier.as_ref());
    let mut links = Vec::new();
    if let Some(propagated) = source
        && let Some(client) = extract_carrier(propagated)
    {
        links.push(Link::with_context(client));
    }
    let transport = trace
        .mcp_transport
        .as_ref()
        .filter(|candidate| candidate.span.context.is_valid())
        .map(|candidate| &candidate.span.context)
        .or_else(|| {
            trace
                .root
                .as_ref()
                .filter(|root| root.context.is_valid())
                .map(|root| &root.context)
        });
    let context = transport.map_or_else(Context::new, |parent| {
        Context::new().with_remote_span_context(parent.clone())
    });
    (context, links)
}

#[path = "mcp_context/tests.rs"]
mod tests;
