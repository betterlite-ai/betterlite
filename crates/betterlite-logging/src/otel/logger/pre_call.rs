use betterlite_models::decisions::Json;
use opentelemetry::trace::TraceContextExt as _;

use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::PreCallEvent;
use crate::otel::emitter::span_emission::StartOptions;
use crate::otel::logger::v2::{OpenCall, OpenTelemetryV2, detached_context, request_trace_links};
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::metadata::wire::callback::Callback;
use crate::otel::vocabulary::registry::SpanRole;

// litellm.integrations.otel.logger.OpenTelemetryV2.log_pre_api_call: open the LLM-call span at the
// boundary when a recordable parent is visible, else record only the start time for the async
// callback to create it against its own worker context
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
    let Some(call_id) = call.call_id else {
        return Ok(());
    };
    // idempotent: a retried call may re-enter pre_call with the same call id; keep the first span
    if logger.open_calls().contains(&call_id) {
        return Ok(());
    }
    let start_time = std::time::SystemTime::now();
    let trace = logger.trace.trace_context();
    let parent = trace.resolve_request_span_context();
    // is_recordable_span(get_current_span(parent)): only an anchored or ambient parent can carry a
    // boundary span, and the thread-pool case defers to the close callback instead
    if !parent.span().span_context().is_valid() {
        logger.store_open_call(
            call_id,
            OpenCall {
                span: None,
                start_time,
                provider: None,
            },
        );
        return Ok(());
    }
    let dynamic_params: Json = serde_json::to_value(&call.dynamic_params)
        .map_err(callback_failed)
        .and_then(|value| serde_json::from_value(value).map_err(callback_failed))?;
    // not routed on the deferred path: creating or touching a tenant provider there would evict
    // idle ones even though close routes afresh
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
    let span = logger.emitter.start_span(
        SpanRole::LlmCall,
        &call.provisional_span_name,
        &StartOptions {
            parent: Some(&parent_context),
            start_time: Some(start_time),
            tracer: Some(&route.tracer),
            links: &links,
        },
    );
    logger.store_open_call(
        call_id,
        OpenCall {
            span: Some(Box::new(span)),
            start_time,
            provider: route.provider,
        },
    );
    Ok(())
}

fn callback_failed(error: impl std::fmt::Display) -> PrePostError {
    PrePostError::CallbackFailed {
        message: error.to_string(),
    }
}
