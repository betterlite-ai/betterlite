use std::collections::BTreeMap;
use std::time::SystemTime;

use opentelemetry::trace::SpanContext;
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::otel::emitter::span_emission::EmitOptions;
use crate::otel::logger::v2::detached_context;
use crate::otel::mappers::base::SpanData;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::post_response::{
    ServiceSpanTiming, resolve_service_span_context,
};
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::vocabulary::resolvers::span_role_for_service;

#[derive(Clone, Copy)]
pub struct ServiceHook<'hook> {
    pub payload: &'hook ServiceLoggerPayload,
    pub parent_otel_span: Option<&'hook SpanContext>,
    pub start_time: Option<SystemTime>,
    pub timing: ServiceSpanTiming,
    pub event_metadata: Option<&'hook BTreeMap<String, OtelModelValue>>,
}

// litellm.integrations.otel.logger.OpenTelemetryV2.async_service_failure_hook: `error or "error"` stamps a missing text
pub fn async_service_failure_hook(
    callback: &V2Callback,
    hook: &ServiceHook<'_>,
    error: Option<&str>,
) -> Option<SdkSpan> {
    let override_text = match error {
        Some(text) if !text.is_empty() => text,
        Some(_) | None => "error",
    };
    emit_service(callback, hook, Some(override_text))
}

// litellm.integrations.otel.logger.OpenTelemetryV2._emit_service, with the success hook's empty override
pub fn emit_service(
    callback: &V2Callback,
    hook: &ServiceHook<'_>,
    error_override: Option<&str>,
) -> Option<SdkSpan> {
    let metadata = hook.event_metadata.cloned().unwrap_or_default();
    let mut data = ServiceSpanData::from_payload(hook.payload, &metadata);
    let role = span_role_for_service(&data.service_name)?;
    if error_override.is_none()
        && hook.start_time.is_none()
        && hook.timing.end_time.is_none()
        && hook.parent_otel_span.is_none()
    {
        return None;
    }
    if let Some(text) = error_override
        && data.error.is_none()
    {
        data.error = Some(SpanError {
            error_type: None,
            message: Some(text.to_owned()),
            code: None,
            stack_trace: None,
            llm_provider: None,
        });
    }
    let (parent, links) = resolve_service_span_context(hook.parent_otel_span, &hook.timing);
    // the ParentBased sampler drops a child of an invalid active span; a linked span starts detached (docs/deviations/B3853.yaml)
    let mut rooted = parent;
    if !links.is_empty() {
        rooted = detached_context(&rooted);
    }
    let options = EmitOptions {
        parent: Some(&rooted),
        start_time: hook.start_time,
        end_time: hook.timing.end_time,
        tracer: None,
        links: &links,
    };
    callback
        .logger
        .emitter
        .emit(role, &SpanData::Service(&data), &options)
}
