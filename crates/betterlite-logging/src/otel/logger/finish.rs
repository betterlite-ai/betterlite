use std::sync::Arc;
use std::time::SystemTime;

use opentelemetry::trace::Span as _;
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::logger::call_context::{CallContext, SpanTimes, seed_identity_baggage};
use crate::otel::logger::v2::{OpenTelemetryV2, detached_context, request_trace_links};
use crate::otel::mappers::base::SpanData;
use crate::otel::model::metadata::wire::MetadataError;
use crate::otel::model::payloads::RequestIdentity;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::{
    MCPListToolsSpanData, MCPToolCallSpanData, PayloadKind,
};
use crate::otel::plumbing::context::mcp_context::resolve_mcp_span_context;
use crate::otel::plumbing::exporters::ExporterError;
use crate::otel::vocabulary::registry::SpanRole;

#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Routing(#[from] ExporterError),
}

// litellm.integrations.otel.logger.OpenTelemetryV2._emit_mcp_tool_call and _emit_mcp_list_tools:
// an MCP call has no pre_call carrier, so the whole CLIENT span is emitted here, deduped on the call
// id; the answer is whether the event was an MCP one, which skips the LLM-call path
pub fn emit_mcp(
    logger: &OpenTelemetryV2,
    call: &CallContext,
    times: SpanTimes,
) -> Result<bool, FinishError> {
    let Some(payload) = call.event.payload.as_ref() else {
        return Ok(false);
    };
    match PayloadKind::from_payload(payload) {
        PayloadKind::McpToolCall => {
            let data = MCPToolCallSpanData::from_standard_logging_payload(
                payload,
                logger.config.capture_span_content(),
            )?;
            emit_mcp_span(
                logger,
                SpanRole::McpToolCall,
                &SpanData::MCPToolCall(&data),
                &data.identity,
                call,
                times,
            )?;
            Ok(true)
        }
        PayloadKind::McpListTools => {
            let data = MCPListToolsSpanData::from_standard_logging_payload(payload)?;
            emit_mcp_span(
                logger,
                SpanRole::McpListTools,
                &SpanData::MCPListTools(&data),
                &data.identity,
                call,
                times,
            )?;
            Ok(true)
        }
        PayloadKind::Inference => Ok(false),
    }
}

// litellm.integrations.otel.logger.OpenTelemetryV2._close_llm_call: pop the carrier as the dedup
// between the success and failure paths, then finish it; a call with no carrier emits only when a
// payload proves it ran, so a gate rejection records no phantom span
pub fn close_llm_call(
    logger: &OpenTelemetryV2,
    call: &CallContext,
    times: SpanTimes,
) -> Result<Option<SdkSpan>, FinishError> {
    let carrier = { logger.open_calls().pop(call.event.call_id.as_deref()) };
    let Some(carrier) = carrier else {
        if call.event.is_no_upstream_call
            || !call.event.upstream_started
            || call.event.payload.is_none()
        {
            return Ok(None);
        }
        return finish_carrier(logger, None, None, call, times);
    };
    let result = finish_carrier(
        logger,
        carrier.span.map(|span| *span),
        Some(carrier.start_time),
        call,
        times,
    );
    // after the span ended, so a release-triggered provider shutdown force-flushes it out
    logger.tenants.release(carrier.provider.as_ref());
    result
}

// litellm.integrations.otel.logger.OpenTelemetryV2._finish_carrier
fn finish_carrier(
    logger: &OpenTelemetryV2,
    span: Option<SdkSpan>,
    start_time: Option<SystemTime>,
    call: &CallContext,
    times: SpanTimes,
) -> Result<Option<SdkSpan>, FinishError> {
    let Some(payload) = call.event.payload.as_ref() else {
        let Some(mut span) = span else {
            return Ok(None);
        };
        // the payload never materialized: end the boundary span under its provisional name so it is
        // not leaked, and mark the call emitted so a later close cannot re-emit it deferred
        logger
            .emitter
            .mark_emitted(call.event.call_id.as_deref(), SpanRole::LlmCall);
        match times.end {
            Some(end) => span.end_with_timestamp(end),
            None => span.end(),
        }
        return Ok(Some(span));
    };
    let trace = logger.trace.trace_context();
    let data = LLMCallSpanData::from_standard_logging_payload(
        payload,
        LLMCallOptions {
            capture_content: logger.config.capture_span_content(),
            time_to_first_chunk_seconds: call.event.time_to_first_chunk_seconds,
            request_route: trace.root.and_then(|root| root.http_route),
            trace: call.event.trace.clone(),
            session_id: call.event.session_id.clone(),
        },
    )?;
    if let Some(mut span) = span {
        // born at the boundary: stamp the payload's attributes, set status, and end it; its parent
        // was captured at creation from the anchored request context
        logger
            .emitter
            .mark_emitted(call.event.call_id.as_deref(), SpanRole::LlmCall);
        logger.emitter.finish_span(
            SpanRole::LlmCall,
            &mut span,
            &SpanData::LLMCall(&data),
            times.end,
        );
        return Ok(Some(span));
    }
    // deferred: pre_call saw no recordable parent, so create the span now against the request's
    // anchored root, seeding identity Baggage so the span is labelled consistently
    let route = logger.tenants.route_for(
        &logger.tracer,
        Some(call.dynamic_params.as_ref()),
        call.event.auth_metadata.as_ref(),
        &trace.destinations,
    )?;
    let seeded = seed_identity_baggage(
        &logger.config,
        &data.identity,
        Some(&data.request_model),
        trace.resolve_request_span_context(),
    );
    let (parent, links) = if route.detached {
        (detached_context(&seeded), request_trace_links(&seeded))
    } else {
        (seeded, Vec::new())
    };
    let emitted = logger.emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(&data),
        &EmitOptions {
            parent: Some(&parent),
            start_time: start_time.or(times.start),
            end_time: times.end,
            tracer: Some(&route.tracer),
            links: &links,
        },
    );
    logger.tenants.release(route.provider.as_ref());
    Ok(emitted)
}

// both MCP events share this: drop a stray LLM carrier that a mis-fired pre_call left for this id,
// route, seed identity Baggage, nest under the message's transport span with the propagated context
// linked, and root a detached span's own trace with a link back
fn emit_mcp_span(
    logger: &OpenTelemetryV2,
    role: SpanRole,
    data: &SpanData<'_>,
    identity: &RequestIdentity,
    call: &CallContext,
    times: SpanTimes,
) -> Result<(), FinishError> {
    let stray = { logger.open_calls().pop(identity.call_id.as_deref()) };
    if let Some(stray) = stray.as_ref() {
        logger.tenants.release(stray.provider.as_ref());
    }
    let trace = logger.trace.trace_context();
    let route = logger.tenants.route_for(
        &logger.tracer,
        None,
        call.event.auth_metadata.as_ref(),
        &trace.destinations,
    )?;
    let (parent, propagated) = resolve_mcp_span_context(&trace, None);
    let seeded = seed_identity_baggage(&logger.config, identity, None, parent);
    let (parent_context, links) = if route.detached {
        let mut links = propagated;
        links.extend(request_trace_links(&seeded));
        (detached_context(&seeded), links)
    } else {
        (seeded, propagated)
    };
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
