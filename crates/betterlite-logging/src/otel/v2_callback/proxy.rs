use opentelemetry::trace::Span as _;

use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::emitter::error_stamping::{ErrorStamp, stamp_error};
use crate::otel::plumbing::context::context_slots::{AnchoredSpan, RequestTraceContext};
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::span_error_from_exception;
use crate::otel::vocabulary::constants::attribute_keys::error as error_keys;

// litellm.integrations.otel.logger.OpenTelemetryV2.create_litellm_proxy_request_started_span
pub fn mark_request_root_span(
    callback: &V2Callback,
    span: &LiveSpan,
    http_route: Option<&str>,
    trace: &mut RequestTraceContext,
) {
    // set_request_root_span no-ops for a span whose context is not valid, as is_recordable_span reads it
    if !span.context.is_valid() {
        return;
    }
    callback.roots.register(span);
    let inherited = trace.root.as_ref().and_then(|root| root.http_route.clone());
    let route = http_route.map(str::to_owned).or(inherited);
    trace.anchor_request_root(AnchoredSpan {
        context: span.context.clone(),
        http_route: route,
    });
}

// litellm.integrations.otel.logger.OpenTelemetryV2.record_error_attributes_on_span
pub fn record_error_on_span(
    callback: &V2Callback,
    span: Option<&LiveSpan>,
    exception: Option<&ExceptionFacts>,
    status_code: u16,
) {
    let Some(live) = span.filter(|live| live.lock().is_recording()) else {
        return;
    };
    let error = span_error_from_exception(&callback.error_info, exception, Some(status_code), None);
    let stamp = ErrorStamp {
        record_event: !live.has_attribute(error_keys::TYPE),
        set_status: true,
    };
    let _stamped = stamp_error(&mut *live.lock(), &error, stamp);
}

// litellm.integrations.otel.logger.OpenTelemetryV2.async_post_call_failure_hook
pub fn async_post_call_failure_hook(
    callback: &V2Callback,
    exception: Option<&ExceptionFacts>,
    traceback_str: Option<&str>,
    parent_span: Option<&LiveSpan>,
    trace: &RequestTraceContext,
) {
    let transport = trace
        .mcp_message_transport_span()
        .and_then(|context| callback.roots.get(context.span_id()));
    let anchor = trace.root.as_ref().map(|root| root.context.span_id());
    let root_span = anchor.and_then(|id| callback.roots.get(id));
    let target = transport.or(root_span).or_else(|| parent_span.cloned());
    let Some(live) = target.filter(|live| live.lock().is_recording()) else {
        return;
    };
    let error = span_error_from_exception(&callback.error_info, exception, None, traceback_str);
    let _stamped = stamp_error(&mut *live.lock(), &error, ErrorStamp::FULL);
}
