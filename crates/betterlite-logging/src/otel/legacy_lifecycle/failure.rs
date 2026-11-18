use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status};

use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::span_time::to_ns;
use crate::otel::plumbing::context::parent_span::ParentSpan;
use crate::otel::v2_callback::live_span::LiveSpan;

use super::error::LegacyLifecycleError;
use super::lifecycle::{LegacyLifecycle, LifecycleCall};
use super::{context, recording, tracer};
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

pub(super) fn failure_primary(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    parent: &ParentSpan,
    parent_context: &Context,
    capture_mode: CaptureMode,
) -> Result<Option<LiveSpan>, LegacyLifecycleError> {
    if parent.span.is_none() || lifecycle.use_request_span {
        let tracer = tracer::tracer_for_request(lifecycle, call.kwargs)?;
        let span = context::start_primary_span(lifecycle, &tracer, call, parent_context);
        span.lock().set_status(Status::error(""));
        recording::apply_attributes(lifecycle, &span, call, capture_mode);
        recording::record_exception(&span, call);
        span.lock()
            .end_with_timestamp(context::instant_from_ns(to_ns(
                call.end_time,
                lifecycle.now_ns(),
            )));
        return Ok(Some(span));
    }
    // the shallow path records the failure on the parent span itself
    if let Some(span) = parent
        .span
        .as_ref()
        .and_then(|context| lifecycle.live.get(context.span_id()))
    {
        let recording = span.lock().is_recording();
        if recording {
            span.lock().set_status(Status::error(""));
            recording::apply_attributes(lifecycle, &span, call, capture_mode);
            recording::record_exception(&span, call);
        }
    }
    Ok(None)
}

pub(super) fn close_proxy(
    lifecycle: &LegacyLifecycle,
    call: &LifecycleCall<'_>,
    parent: Option<&LiveSpan>,
) {
    // no is_recording guard: the failure hook may close this proxy again
    if let Some(span) = parent
        && context::name_of(span).as_deref() == Some(PROXY_REQUEST_SPAN_NAME)
    {
        span.lock()
            .end_with_timestamp(context::instant_from_ns(to_ns(
                call.end_time,
                lifecycle.now_ns(),
            )));
    }
}
