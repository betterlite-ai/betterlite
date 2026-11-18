use super::error::LegacyLifecycleError;
use super::lifecycle::{
    DEFAULT_CALLBACK_NAME, LegacyLifecycle, LifecycleCall, LifecycleEvent, litellm_params,
};
use crate::exporter_options::failure_counter::{CallbackFailureCounter, handle_callback_failure};
use crate::otel::legacy_policy::semconv_attrs::{
    gen_ai_operation_name, semconv_latest_experimental,
};
use crate::otel::legacy_policy::span_time::{span_name, to_ns};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::v2_callback::live_span::LiveSpan;
use opentelemetry::Context;
use opentelemetry::trace::{SpanKind, TraceContextExt as _, Tracer as _};
use opentelemetry_sdk::trace::SdkTracer;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// an ended SDK span reports no data, so only a recording span answers with its name
#[must_use]
pub fn name_of(span: &LiveSpan) -> Option<String> {
    let exported = span.lock().exported_data();
    if let Some(data) = exported {
        return Some(data.name.to_string());
    }
    None
}

// the clock source is whole nanoseconds, and a pre-epoch value still maps to a SystemTime
#[must_use]
pub fn instant_from_ns(nanoseconds: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(nanoseconds.unsigned_abs());
    if nanoseconds.is_negative() {
        UNIX_EPOCH.checked_sub(magnitude).unwrap_or(UNIX_EPOCH)
    } else {
        UNIX_EPOCH.checked_add(magnitude).unwrap_or(UNIX_EPOCH)
    }
}

// a guardrail is parented to the span the request landed on, never to an orphaned root
#[must_use]
pub fn resolve_guardrail_context(
    primary: Option<&LiveSpan>,
    parent_span: Option<&LiveSpan>,
    fallback: Option<&Context>,
) -> Context {
    if let Some(span) = primary.or(parent_span) {
        return Context::new().with_remote_span_context(span.context.clone());
    }
    fallback.cloned().unwrap_or_default()
}

#[must_use]
pub fn standard_logging_object(kwargs: &MetadataMap) -> Option<&MetadataMap> {
    kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
}

pub(super) fn count_failure(lifecycle: &LegacyLifecycle) {
    let mut counters: Vec<&dyn CallbackFailureCounter> =
        Vec::with_capacity(lifecycle.failure_counters.len());
    for counter in &lifecycle.failure_counters {
        counters.push(counter.as_ref());
    }
    handle_callback_failure(
        lifecycle
            .callback_name
            .as_deref()
            .unwrap_or(DEFAULT_CALLBACK_NAME),
        &counters,
    );
}

pub(super) fn parent(
    lifecycle: &LegacyLifecycle,
    kwargs: &MetadataMap,
) -> Result<ParentSpan, LegacyLifecycleError> {
    if lifecycle.config.ignore_context_propagation {
        return Ok(ParentSpan::default());
    }
    Ok(super::parent::from_kwargs(kwargs)?)
}

pub(super) fn start_primary_span(
    lifecycle: &LegacyLifecycle,
    tracer: &SdkTracer,
    call: &LifecycleCall<'_>,
    context: &Context,
) -> LiveSpan {
    let params = litellm_params(call.kwargs).cloned().unwrap_or_default();
    let semconv = semconv_latest_experimental(&lifecycle.config);
    let operation = gen_ai_operation_name(
        call.kwargs
            .get("call_type")
            .and_then(OtelModelValue::as_str),
    );
    let model = call.kwargs.get("model").and_then(OtelModelValue::as_str);
    let name = span_name(&params, operation, model, semconv);
    let mut builder = tracer
        .span_builder(name)
        .with_start_time(instant_from_ns(to_ns(call.start_time, lifecycle.now_ns())));
    if semconv {
        builder = builder.with_kind(SpanKind::Client);
    }
    LiveSpan::from(tracer.build_with_context(builder, context))
}

pub(super) struct RequestSpans {
    pub(super) primary: Option<LiveSpan>,
    pub(super) parent: Option<LiveSpan>,
    pub(super) fallback: Option<Context>,
}

pub(super) fn request_spans(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    event: LifecycleEvent,
) -> Result<RequestSpans, LegacyLifecycleError> {
    let parent = parent(lifecycle, call.kwargs)?;
    let live_parent = parent
        .span
        .as_ref()
        .and_then(|span| lifecycle.live.get(span.span_id()));
    let capture = lifecycle.capture_policy.resolve(call.flags);
    let parent_context = parent.context.clone().unwrap_or_default();
    let primary = match event {
        LifecycleEvent::Success => {
            super::success::success_primary(lifecycle, call, &parent, &parent_context, capture)?
        }
        LifecycleEvent::Failure => {
            super::failure::failure_primary(lifecycle, call, &parent, &parent_context, capture)?
        }
    };
    Ok(RequestSpans {
        primary,
        parent: live_parent,
        fallback: parent.context,
    })
}
