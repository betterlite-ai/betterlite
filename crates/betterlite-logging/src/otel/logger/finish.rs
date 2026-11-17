use std::time::SystemTime;

use opentelemetry::trace::Span as _;
use opentelemetry_sdk::trace::Span as SdkSpan;

use self::mcp::{McpEmission, emit_mcp_span};
use crate::otel::emitter::span_emission::{EmitOptions, StartOptions};
use crate::otel::logger::call_context::{CallContext, SpanTimes, seed_identity_baggage};
use crate::otel::logger::v2::{
    BoundarySpan, OpenTelemetryV2, detached_context, request_trace_links,
};
use crate::otel::mappers::base::SpanData;
use crate::otel::model::metadata::wire::MetadataError;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::{
    MCPListToolsSpanData, MCPToolCallSpanData, PayloadKind,
};
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::plumbing::exporters::ExporterError;
use crate::otel::vocabulary::registry::SpanRole;

mod mcp;

#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    #[error(transparent)]
    Routing(#[from] ExporterError),
}

// litellm.integrations.otel.logger.OpenTelemetryV2._emit_mcp_tool_call and _emit_mcp_list_tools: no pre_call carrier exists, so the whole CLIENT span is emitted here, deduped on the call id
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
            let emission = McpEmission {
                role: SpanRole::McpToolCall,
                data: &SpanData::MCPToolCall(&data),
                identity: &data.identity,
                call,
                times,
            };
            emit_mcp_span(logger, emission)?;
        }
        PayloadKind::McpListTools => {
            let data = MCPListToolsSpanData::from_standard_logging_payload(payload)?;
            let emission = McpEmission {
                role: SpanRole::McpListTools,
                data: &SpanData::MCPListTools(&data),
                identity: &data.identity,
                call,
                times,
            };
            emit_mcp_span(logger, emission)?;
        }
        PayloadKind::Inference => return Ok(false),
    }
    Ok(true)
}

// litellm.integrations.otel.logger.OpenTelemetryV2._close_llm_call: pop the carrier as the success/failure dedup, then finish it; a carrier-less call emits only when a payload proves it ran
pub fn close_llm_call(
    logger: &OpenTelemetryV2,
    call: &CallContext,
    times: SpanTimes,
) -> Result<Option<SdkSpan>, FinishError> {
    let stored = {
        let mut calls = logger.open_calls();
        call.event
            .call_id
            .as_deref()
            .and_then(|id| calls.calls.shift_remove(id))
    };
    let Some(carrier) = stored else {
        if call.event.is_no_upstream_call
            || !call.event.upstream_started
            || call.event.payload.is_none()
        {
            return Ok(None);
        }
        let close = CarrierClose {
            boundary: None,
            start_time: None,
            call,
            times,
        };
        return finish_carrier(logger, close);
    };
    let close = CarrierClose {
        boundary: carrier.boundary,
        start_time: Some(carrier.start_time),
        call,
        times,
    };
    let result = finish_carrier(logger, close);
    // after the span ended, so a release-triggered provider shutdown force-flushes it out
    logger.tenants.release(carrier.provider.as_ref());
    result
}

// the close state a carrier hands on: the boundary state, if pre_call saw a recordable parent, and the start time a deferred span uses
struct CarrierClose<'call> {
    boundary: Option<BoundarySpan>,
    start_time: Option<SystemTime>,
    call: &'call CallContext,
    times: SpanTimes,
}

// litellm.integrations.otel.logger.OpenTelemetryV2._finish_carrier
fn finish_carrier(
    logger: &OpenTelemetryV2,
    close: CarrierClose<'_>,
) -> Result<Option<SdkSpan>, FinishError> {
    let Some(payload) = close.call.event.payload.as_ref() else {
        return Ok(end_without_payload(
            logger,
            close.boundary,
            close.start_time,
            close.call,
            close.times,
        ));
    };
    let trace = logger.trace.trace_context();
    let data = llm_call_data(logger, close.call, payload, &trace)?;
    let Some(open) = close.boundary.as_ref() else {
        return emit_deferred(logger, &close, &data, &trace);
    };
    Ok(finish_open_span(
        logger,
        open,
        close.start_time,
        &data,
        close.times,
    ))
}

// the payload never materialized: open the boundary span at its captured start and end it under its provisional name, then mark the call emitted so a later close cannot re-emit it deferred
fn end_without_payload(
    logger: &OpenTelemetryV2,
    boundary: Option<BoundarySpan>,
    start_time: Option<SystemTime>,
    call: &CallContext,
    times: SpanTimes,
) -> Option<SdkSpan> {
    let open = boundary?;
    // the marker only has to exist before a later close looks; this path has nothing to do with its answer
    let _marked = logger
        .emitter
        .mark_emitted(call.event.call_id.as_deref(), SpanRole::LlmCall);
    let mut span = logger.emitter.start_span(
        SpanRole::LlmCall,
        &call.event.provisional_span_name,
        &StartOptions {
            parent: Some(&open.parent),
            start_time,
            tracer: Some(&open.tracer),
            links: &open.links,
        },
    );
    match times.end {
        Some(end) => span.end_with_timestamp(end),
        None => span.end(),
    }
    Some(span)
}

fn llm_call_data(
    logger: &OpenTelemetryV2,
    call: &CallContext,
    payload: &Payload,
    trace: &RequestTraceContext,
) -> Result<LLMCallSpanData, FinishError> {
    let options = LLMCallOptions {
        capture_content: logger.config.capture_span_content(),
        time_to_first_chunk_seconds: call.event.time_to_first_chunk_seconds,
        request_route: trace.root.as_ref().and_then(|root| root.http_route.clone()),
        trace: call.event.trace.clone(),
        session_id: call.event.session_id.clone(),
    };
    Ok(LLMCallSpanData::from_standard_logging_payload(
        payload, options,
    )?)
}

// born at the boundary: the payload proves the call ran, so open on the parent, tracer and links pre_call captured, then stamp and end it under the payload model
fn finish_open_span(
    logger: &OpenTelemetryV2,
    boundary: &BoundarySpan,
    start_time: Option<SystemTime>,
    data: &LLMCallSpanData,
    times: SpanTimes,
) -> Option<SdkSpan> {
    logger.emitter.emit(
        SpanRole::LlmCall,
        &SpanData::LLMCall(data),
        &EmitOptions {
            parent: Some(&boundary.parent),
            start_time,
            end_time: times.end,
            tracer: Some(&boundary.tracer),
            links: &boundary.links,
        },
    )
}

// deferred: pre_call saw no recordable parent, so create the span now against the request's anchored root
fn emit_deferred(
    logger: &OpenTelemetryV2,
    close: &CarrierClose<'_>,
    data: &LLMCallSpanData,
    trace: &RequestTraceContext,
) -> Result<Option<SdkSpan>, FinishError> {
    let call = close.call;
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
        &SpanData::LLMCall(data),
        &EmitOptions {
            parent: Some(&parent),
            start_time: close.start_time.or(close.times.start),
            end_time: close.times.end,
            tracer: Some(&route.tracer),
            links: &links,
        },
    );
    logger.tenants.release(route.provider.as_ref());
    Ok(emitted)
}
