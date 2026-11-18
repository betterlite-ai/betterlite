use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status, TraceContextExt as _, Tracer as _};
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_lifecycle::lifecycle::LegacyLifecycle;
use crate::otel::legacy_policy::error_recording::record_exception_on_span;
use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::logger_runtime::legacy::set_response_status_code;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

use super::error_info;
use super::timing;

// ast-grep-ignore: no-long-names
pub struct ManagementEndpointLoggingPayload {
    pub route: String,
    pub request_data: MetadataMap,
    pub response: Option<MetadataMap>,
    pub exception: Option<ExceptionFacts>,
    pub start_time: SpanTime,
    pub end_time: SpanTime,
}

// ast-grep-ignore: no-long-names
pub fn async_management_endpoint_success_hook(
    lifecycle: &LegacyLifecycle,
    payload: &ManagementEndpointLoggingPayload,
    parent: Option<&LiveSpan>,
) {
    let Some(parent) = parent else {
        return;
    };
    let start_ns = to_ns(payload.start_time, lifecycle.now_ns());
    let end_ns = to_ns(payload.end_time, lifecycle.now_ns());
    let span = start_endpoint_span(lifecycle, payload, parent, start_ns);
    write_call_attributes(&mut *span.lock(), "request", &payload.request_data);
    if let Some(response) = &payload.response {
        write_call_attributes(&mut *span.lock(), "response", response);
    }
    span.lock().set_status(Status::Ok);
    span.lock().end_with_timestamp(instant_from_ns(end_ns));
    set_response_status_code(Some(parent), Some(200));
    parent.lock().set_status(Status::Ok);
    parent.lock().end_with_timestamp(instant_from_ns(end_ns));
}

// ast-grep-ignore: no-long-names
pub fn async_management_endpoint_failure_hook(
    lifecycle: &LegacyLifecycle,
    payload: &ManagementEndpointLoggingPayload,
    parent: Option<&LiveSpan>,
    info: &ErrorInformationConfig,
) {
    let Some(parent) = parent else {
        return;
    };
    let start_ns = timing::whole_second_ns(payload.start_time, lifecycle.now_ns());
    let end_ns = timing::whole_second_ns(payload.end_time, lifecycle.now_ns());
    let span = start_endpoint_span(lifecycle, payload, parent, start_ns);
    write_call_attributes(&mut *span.lock(), "request", &payload.request_data);
    let mut target = span.lock();
    target.safe_set_attribute(
        "exception",
        &OtelModelValue::String(error_info::exception_text(payload.exception.as_ref())),
    );
    target.set_status(Status::error(""));
    target.end_with_timestamp(instant_from_ns(end_ns));
    drop(target);
    let information = error_info::information(info, payload.exception.as_ref(), None);
    parent.lock().set_status(Status::error(""));
    record_exception_on_span(
        &mut *parent.lock(),
        error_info::recorded_exception(payload.exception.as_ref()).as_ref(),
        &error_info::error_payload(&information),
    );
    parent.lock().end_with_timestamp(instant_from_ns(end_ns));
}

// ast-grep-ignore: no-long-names
fn start_endpoint_span(
    lifecycle: &LegacyLifecycle,
    payload: &ManagementEndpointLoggingPayload,
    parent: &LiveSpan,
    start_ns: i64,
) -> LiveSpan {
    let context = Context::new().with_remote_span_context(parent.context.clone());
    let builder = lifecycle
        .tracer
        .span_builder(payload.route.clone())
        .with_start_time(instant_from_ns(start_ns));
    LiveSpan::from(lifecycle.tracer.build_with_context(builder, &context))
}

fn write_call_attributes(span: &mut SdkSpan, prefix: &str, fields: &MetadataMap) {
    for (key, value) in fields {
        span.safe_set_attribute(&format!("{prefix}.{key}"), value);
    }
}
