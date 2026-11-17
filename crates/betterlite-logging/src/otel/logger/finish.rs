use std::time::SystemTime;

use opentelemetry::trace::Span as _;
use opentelemetry_sdk::trace::Span as SdkSpan;

use self::mcp::{McpEmission, emit_mcp_span};
use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::logger::call_context::{CallContext, SpanTimes, seed_identity_baggage};
use crate::otel::logger::v2::{OpenTelemetryV2, detached_context, request_trace_links};
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
    let stored = { logger.open_calls().pop(call.event.call_id.as_deref()) };
    let Some(carrier) = stored else {
        if call.event.is_no_upstream_call
            || !call.event.upstream_started
            || call.event.payload.is_none()
        {
            return Ok(None);
        }
        let close = CarrierClose {
            span: None,
            start_time: None,
            call,
            times,
        };
        return finish_carrier(logger, close);
    };
    let close = CarrierClose {
        span: carrier.span.map(|span| *span),
        start_time: Some(carrier.start_time),
        call,
        times,
    };
    let result = finish_carrier(logger, close);
    // after the span ended, so a release-triggered provider shutdown force-flushes it out
    logger.tenants.release(carrier.provider.as_ref());
    result
}

// the close state a carrier hands on: the boundary span, if it opened one, and the start time a deferred span uses
struct CarrierClose<'a> {
    span: Option<SdkSpan>,
    start_time: Option<SystemTime>,
    call: &'a CallContext,
    times: SpanTimes,
}

// litellm.integrations.otel.logger.OpenTelemetryV2._finish_carrier
fn finish_carrier(
    logger: &OpenTelemetryV2,
    close: CarrierClose<'_>,
) -> Result<Option<SdkSpan>, FinishError> {
    let CarrierClose {
        span,
        start_time,
        call,
        times,
    } = close;
    let Some(payload) = call.event.payload.as_ref() else {
        return Ok(end_without_payload(logger, span, call, times));
    };
    let trace = logger.trace.trace_context();
    let data = llm_call_data(logger, call, payload, &trace)?;
    match span {
        Some(span) => Ok(finish_open_span(logger, span, &data, call, times)),
        None => {
            let deferred = DeferredClose {
                data: &data,
                call,
                trace: &trace,
                start_time,
                times,
            };
            emit_deferred(logger, deferred)
        }
    }
}

// the payload never materialized: end the boundary span under its provisional name, and mark the call emitted so a later close cannot re-emit it deferred
fn end_without_payload(
    logger: &OpenTelemetryV2,
    span: Option<SdkSpan>,
    call: &CallContext,
    times: SpanTimes,
) -> Option<SdkSpan> {
    let mut open = span?;
    logger
        .emitter
        .mark_emitted(call.event.call_id.as_deref(), SpanRole::LlmCall);
    match times.end {
        Some(end) => open.end_with_timestamp(end),
        None => open.end(),
    }
    Some(open)
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

// born at the boundary: stamp the payload's attributes, set status, and end it under the parent captured at creation
fn finish_open_span(
    logger: &OpenTelemetryV2,
    mut span: SdkSpan,
    data: &LLMCallSpanData,
    call: &CallContext,
    times: SpanTimes,
) -> Option<SdkSpan> {
    logger
        .emitter
        .mark_emitted(call.event.call_id.as_deref(), SpanRole::LlmCall);
    logger.emitter.finish_span(
        SpanRole::LlmCall,
        &mut span,
        &SpanData::LLMCall(data),
        times.end,
    );
    Some(span)
}

// what the deferred branch routes and emits with: the mapped data, its call, and the request trace
struct DeferredClose<'a> {
    data: &'a LLMCallSpanData,
    call: &'a CallContext,
    trace: &'a RequestTraceContext,
    start_time: Option<SystemTime>,
    times: SpanTimes,
}

// deferred: pre_call saw no recordable parent, so create the span now against the request's anchored root
fn emit_deferred(
    logger: &OpenTelemetryV2,
    close: DeferredClose<'_>,
) -> Result<Option<SdkSpan>, FinishError> {
    let DeferredClose {
        data,
        call,
        trace,
        start_time,
        times,
    } = close;
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
            start_time: start_time.or(times.start),
            end_time: times.end,
            tracer: Some(&route.tracer),
            links: &links,
        },
    );
    logger.tenants.release(route.provider.as_ref());
    Ok(emitted)
}
