use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status, TraceContextExt as _, Tracer as _};

use crate::otel::legacy_attributes::raw_request::set_raw_request_attributes;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::semconv_attrs::semconv_latest_experimental;
use crate::otel::legacy_policy::span_time::to_ns;
use crate::otel::legacy_policy::text_field;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::context::RequestSpans;
use super::error::LegacyLifecycleError;
use super::lifecycle::{LegacyLifecycle, LifecycleCall, litellm_params};
use super::{context, proxy, recording, tracer};

pub const RAW_REQUEST_SPAN_NAME: &str = "raw_gen_ai_request";

pub(super) fn maybe_log_raw_request(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    parent_context: &Context,
) -> Result<(), LegacyLifecycleError> {
    // the raw payload span carries provider-specific keys, which the stable semconv replaces
    if semconv_latest_experimental(&lifecycle.config) {
        return Ok(());
    }
    if !lifecycle
        .capture_policy
        .resolve(call.flags)
        .capture_in_span()
    {
        return Ok(());
    }
    let generation_name = litellm_params(call.kwargs)
        .and_then(|params| params.get("metadata"))
        .and_then(OtelModelValue::as_object)
        .and_then(|metadata| text_field(metadata, "generation_name"));
    let name = generation_name.map_or_else(|| RAW_REQUEST_SPAN_NAME.to_owned(), str::to_owned);
    let tracer = tracer::tracer_for_request(lifecycle, call.kwargs)?;
    let builder = tracer
        .span_builder(name)
        .with_start_time(context::instant_from_ns(to_ns(
            call.start_time,
            lifecycle.now_ns(),
        )));
    let mut span = tracer.build_with_context(builder, parent_context);
    span.set_status(Status::Ok);
    set_raw_request_attributes(&mut span, call.kwargs);
    recording::write_team_attributes_from_log(&mut span, call.kwargs);
    span.end_with_timestamp(context::instant_from_ns(to_ns(
        call.end_time,
        lifecycle.now_ns(),
    )));
    Ok(())
}

pub(super) fn success_primary(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    parent: &ParentSpan,
    parent_context: &Context,
    capture_mode: CaptureMode,
) -> Result<Option<LiveSpan>, LegacyLifecycleError> {
    if parent.span.is_none() || lifecycle.use_request_span {
        let tracer = tracer::tracer_for_request(lifecycle, call.kwargs)?;
        let span = context::start_primary_span(lifecycle, &tracer, call, parent_context);
        span.lock().set_status(Status::Ok);
        recording::apply_attributes(lifecycle, &span, call, capture_mode);
        span.lock()
            .end_with_timestamp(context::instant_from_ns(to_ns(
                call.end_time,
                lifecycle.now_ns(),
            )));
        let raw_context = Context::new().with_remote_span_context(span.context.clone());
        maybe_log_raw_request(lifecycle, call, &raw_context)?;
        return Ok(Some(span));
    }
    // a parent span keeps the hierarchy shallow: its own span carries the request attributes
    if let Some(span) = parent
        .span
        .as_ref()
        .and_then(|context| lifecycle.live.get(context.span_id()))
    {
        span.lock().set_status(Status::Ok);
        recording::apply_attributes(lifecycle, &span, call, capture_mode);
    }
    maybe_log_raw_request(lifecycle, call, parent_context)?;
    Ok(None)
}

pub(super) fn finish_success(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    spans: &RequestSpans,
) -> Result<(), LegacyLifecycleError> {
    let span_context = spans
        .primary
        .as_ref()
        .or(spans.parent.as_ref())
        .map(|span| &span.context);
    recording::record_success(lifecycle, call, span_context)?;
    if let Some(span) = &spans.parent {
        let recording = span.lock().is_recording();
        if context::name_of(span).as_deref() == Some(PROXY_REQUEST_SPAN_NAME) && recording {
            proxy::close_proxy_ok(lifecycle, span, call.end_time);
        }
    }
    proxy::stamp_proxy_team(lifecycle, call.kwargs);
    proxy::end_proxy_from_kwargs(lifecycle, call);
    Ok(())
}
