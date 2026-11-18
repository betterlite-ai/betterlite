use opentelemetry::trace::{Span as _, Status};

use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::logger_runtime::legacy::set_response_status_code;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::plumbing::context::parent_span::span_context_from_value;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::lifecycle::{LegacyLifecycle, LifecycleCall};
use super::parent;
use super::{context, recording};

pub(super) fn stamp_proxy_team(lifecycle: &LegacyLifecycle, kwargs: &MetadataMap) {
    if let Some(span) = recording_proxy_span(lifecycle, kwargs, false) {
        recording::write_team_attributes_from_log(&mut *span.lock(), kwargs);
    }
}

pub(super) fn end_proxy_from_kwargs(lifecycle: &LegacyLifecycle, call: &LifecycleCall<'_>) {
    if let Some(span) = recording_proxy_span(lifecycle, call.kwargs, true) {
        close_proxy_ok(lifecycle, &span, call.end_time);
    }
}

pub(super) fn recording_proxy_span(
    lifecycle: &LegacyLifecycle,
    kwargs: &MetadataMap,
    with_litellm_metadata: bool,
) -> Option<LiveSpan> {
    let carrier = parent::proxy_carrier(kwargs, with_litellm_metadata)?;
    let context = span_context_from_value(carrier)?;
    let span = lifecycle.live.get(context.span_id())?;
    if context::name_of(&span).as_deref() != Some(PROXY_REQUEST_SPAN_NAME)
        || !span.lock().is_recording()
    {
        return None;
    }
    Some(span)
}

pub(super) fn close_proxy_ok(lifecycle: &LegacyLifecycle, span: &LiveSpan, end_time: SpanTime) {
    set_response_status_code(Some(span), Some(200));
    span.lock().set_status(Status::Ok);
    span.lock()
        .end_with_timestamp(context::instant_from_ns(to_ns(
            end_time,
            lifecycle.now_ns(),
        )));
}
