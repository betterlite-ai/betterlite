use opentelemetry::Context;
use opentelemetry::trace::Link;

use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::logger::call_context::{CallContext, SpanTimes, seed_identity_baggage};
use crate::otel::logger::v2::{OpenTelemetryV2, detached_context, request_trace_links};
use crate::otel::mappers::base::SpanData;
use crate::otel::model::payloads::RequestIdentity;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::context::mcp_context::resolve_mcp_span_context;
use crate::otel::plumbing::routing::cache::TenantRoute;
use crate::otel::vocabulary::registry::SpanRole;

use super::FinishError;

// one MCP span as LiteLLM's _emit_mcp_tool_call and _emit_mcp_list_tools build it: the role, the mapped data, the identity and the call
#[derive(Clone, Copy)]
pub(super) struct McpEmission<'emission, 'data> {
    pub(super) role: SpanRole,
    pub(super) data: &'emission SpanData<'data>,
    pub(super) identity: &'emission RequestIdentity,
    pub(super) call: &'emission CallContext,
    pub(super) times: SpanTimes,
}

// both MCP events share this: drop a stray carrier, route, seed Baggage, nest under the transport span, and root a detached span with a link back
pub(super) fn emit_mcp_span(
    logger: &OpenTelemetryV2,
    emission: McpEmission<'_, '_>,
) -> Result<(), FinishError> {
    let McpEmission {
        role,
        data,
        identity,
        call,
        times,
    } = emission;
    drop_stray_carrier(logger, identity);
    let trace = logger.trace.trace_context();
    let route = logger.tenants.route_for(
        &logger.tracer,
        None,
        call.event.auth_metadata.as_ref(),
        &trace.destinations,
    )?;
    let (parent_context, links) = mcp_parent(logger, &route, identity, &trace);
    logger.emitter.emit(
        role,
        data,
        &EmitOptions {
            parent: Some(&parent_context),
            start_time: times.start,
            end_time: times.end,
            tracer: Some(&route.tracer),
            links: &links,
        },
    );
    logger.tenants.release(route.provider.as_ref());
    Ok(())
}

// a carrier a misfired pre_call left for this call id would otherwise linger until evicted; drop it so it is never a phantom LLM span
fn drop_stray_carrier(logger: &OpenTelemetryV2, identity: &RequestIdentity) {
    let stray = {
        let mut calls = logger.open_calls();
        identity
            .call_id
            .as_deref()
            .and_then(|id| calls.calls.shift_remove(id))
    };
    if let Some(carrier) = stray.as_ref() {
        logger.tenants.release(carrier.provider.as_ref());
    }
}

// the MCP span's parent and links: the message's transport span, or a detached route rooted in its own trace and linked back
fn mcp_parent(
    logger: &OpenTelemetryV2,
    route: &TenantRoute,
    identity: &RequestIdentity,
    trace: &RequestTraceContext,
) -> (Context, Vec<Link>) {
    let (parent, propagated) = resolve_mcp_span_context(trace, None);
    let seeded = seed_identity_baggage(&logger.config, identity, None, parent);
    if !route.detached {
        return (seeded, propagated);
    }
    let mut links = propagated;
    links.extend(request_trace_links(&seeded));
    (detached_context(&seeded), links)
}
