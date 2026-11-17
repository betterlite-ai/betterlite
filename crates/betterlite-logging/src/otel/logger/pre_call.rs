use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use opentelemetry::trace::TraceContextExt as _;
use serde::Deserialize;

use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::PreCallEvent;
use crate::otel::emitter::span_emission::StartOptions;
use crate::otel::logger::v2::{OpenCall, OpenTelemetryV2, detached_context, request_trace_links};
use crate::otel::vocabulary::registry::SpanRole;
use crate::otel::vocabulary::resolvers::resolve_operation;

// the pre_call boundary hands over ``model_call_details``; only the keys the lifecycle reads are
// projected, so the boundary's own timestamp and payload keys never have to be understood here
#[derive(Debug, Deserialize)]
struct BoundaryDetails {
    model: Option<String>,
    call_type: Option<String>,
    litellm_call_id: Option<String>,
    #[serde(default)]
    litellm_no_upstream_llm_call: bool,
    standard_callback_dynamic_params: Option<Json>,
    litellm_params: Option<BoundaryParams>,
}

#[derive(Debug, Deserialize)]
struct BoundaryParams {
    metadata: Option<BoundaryMetadata>,
    litellm_metadata: Option<BoundaryMetadata>,
}

#[derive(Debug, Deserialize)]
struct BoundaryMetadata {
    user_api_key_auth_metadata: Option<BTreeMap<String, String>>,
}

// LiteLLM's auth_metadata kwargs read: litellm_params.metadata first, then its litellm_metadata
fn auth_metadata(params: Option<&BoundaryParams>) -> Option<BTreeMap<String, String>> {
    let params = params?;
    [params.metadata.as_ref(), params.litellm_metadata.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|metadata| metadata.user_api_key_auth_metadata.as_ref())
        .find(|entries| !entries.is_empty())
        .cloned()
}

// litellm.integrations.otel.logger.OpenTelemetryV2.log_pre_api_call: open the LLM-call span at the
// boundary when a recordable parent is visible, else record only the start time for the async
// callback to create it against its own worker context
pub fn log_pre_api_call(
    logger: &OpenTelemetryV2,
    event: &PreCallEvent<'_>,
) -> Result<(), PrePostError> {
    let decoded: BoundaryDetails =
        serde_json::from_value(serde_json::to_value(event.details)?).map_err(callback_failed)?;
    // a synthetic proxy-gate log never made an upstream call, so it gets no span and no carrier
    if decoded.litellm_no_upstream_llm_call {
        return Ok(());
    }
    let Some(call_id) = decoded.litellm_call_id else {
        return Ok(());
    };
    let model = decoded.model.as_deref().unwrap_or(event.model);
    let span_name = format!(
        "{} {model}",
        resolve_operation(decoded.call_type.as_deref().unwrap_or_default())
    )
    .trim()
    .to_owned();
    let auth_metadata = auth_metadata(decoded.litellm_params.as_ref());
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
    // not routed on the deferred path: creating or touching a tenant provider there would evict
    // idle ones even though close routes afresh
    let route = logger
        .tenants
        .route_for(
            &logger.tracer,
            decoded.standard_callback_dynamic_params.as_ref(),
            auth_metadata.as_ref(),
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
        &span_name,
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
