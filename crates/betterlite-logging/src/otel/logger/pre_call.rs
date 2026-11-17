use std::fmt;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use opentelemetry::Context;
use opentelemetry::trace::TraceContextExt as _;

use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::PreCallEvent;
use crate::otel::logger::v2::{
    BoundarySpan, OpenCall, OpenTelemetryV2, detached_context, request_trace_links,
};
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::metadata::wire::callback::Callback;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;

// litellm.integrations.otel.logger.OpenTelemetryV2.log_pre_api_call: open the LLM-call span at the boundary when a recordable parent is visible, else record only the start time for the async close
pub fn log_pre_api_call(
    logger: &OpenTelemetryV2,
    event: &PreCallEvent<'_>,
) -> Result<(), PrePostError> {
    let details = serde_json::to_value(event.details).map_err(callback_failed)?;
    let callback: Callback = serde_json::from_value(details).map_err(callback_failed)?;
    let call = LLMCallEvent::from_dict(&callback).map_err(callback_failed)?;
    // a synthetic proxy-gate log never made an upstream call, so it gets no span and no carrier
    if call.is_no_upstream_call {
        return Ok(());
    }
    let Some(call_id) = call.call_id.clone() else {
        return Ok(());
    };
    // idempotent: a retried call may re-enter pre_call with the same call id; keep the first span
    if logger.open_calls().calls.contains_key(&call_id) {
        return Ok(());
    }
    let start_time = SystemTime::now();
    let trace = logger.trace.trace_context();
    let parent = trace.resolve_request_span_context();
    // is_recordable_span(get_current_span(parent)): only an anchored or ambient parent carries a boundary
    if !parent.span().span_context().is_valid() {
        let carrier = OpenCall {
            boundary: None,
            start_time,
            provider: None,
        };
        logger.store_open_call(call_id, carrier);
        return Ok(());
    }
    let carrier = open_at_boundary(logger, &call, &trace, parent, start_time)?;
    logger.store_open_call(call_id, carrier);
    Ok(())
}

// the boundary arm of OpenTelemetryV2.log_pre_api_call: route here, and keep what the close callback needs to open on the routed tracer, detached for a tenant account
fn open_at_boundary(
    logger: &OpenTelemetryV2,
    call: &LLMCallEvent,
    trace: &RequestTraceContext,
    parent: Context,
    start_time: SystemTime,
) -> Result<OpenCall, PrePostError> {
    let dynamic_params: Json = serde_json::to_value(&call.dynamic_params)
        .map_err(callback_failed)
        .and_then(|value| serde_json::from_value(value).map_err(callback_failed))?;
    let route = logger
        .tenants
        .route_for(
            &logger.tracer,
            Some(&dynamic_params),
            call.auth_metadata.as_ref(),
            &trace.destinations,
        )
        .map_err(callback_failed)?;
    let (parent_context, links) = if route.detached {
        (detached_context(&parent), request_trace_links(&parent))
    } else {
        (parent, Vec::new())
    };
    Ok(OpenCall {
        boundary: Some(BoundarySpan {
            parent: parent_context,
            tracer: route.tracer,
            links,
        }),
        start_time,
        provider: route.provider,
    })
}

fn callback_failed<T>(error: T) -> PrePostError
where
    T: fmt::Display,
{
    PrePostError::CallbackFailed {
        message: error.to_string(),
    }
}
