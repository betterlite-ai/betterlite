use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status, TraceContextExt as _, Tracer as _};

use crate::logging_metadata::error_info::get_error_information;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_lifecycle::guardrail;
use crate::otel::legacy_lifecycle::lifecycle::LegacyLifecycle;
use crate::otel::legacy_lifecycle::parent;
use crate::otel::legacy_policy::annotations::set_team_attributes;
use crate::otel::legacy_policy::error_recording::record_exception_on_span;
use crate::otel::logger_runtime::legacy::set_response_status_code;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

use super::error::ServiceCallbackError;
use super::error_info;
use super::preprocessing::set_preprocessing_duration;

pub const FAILED_REQUEST_SPAN_NAME: &str = "Failed Proxy Server Request";

// litellm.proxy._types.UserAPIKeyAuth, the three fields these hooks read
pub struct ProxyCallKey<'call> {
    pub parent_span: Option<&'call LiveSpan>,
    pub team_id: Option<&'call str>,
    pub team_alias: Option<&'call str>,
}

pub struct PostCallFailure<'call> {
    pub request_data: &'call MetadataMap,
    pub exception: Option<&'call ExceptionFacts>,
    pub key: &'call ProxyCallKey<'call>,
    pub traceback: Option<&'call str>,
    pub info: &'call ErrorInformationConfig,
}

// litellm.integrations.opentelemetry.OpenTelemetry.async_post_call_failure_hook
pub fn async_post_call_failure_hook(
    lifecycle: &LegacyLifecycle,
    call: &PostCallFailure<'_>,
) -> Result<(), ServiceCallbackError> {
    let Some(parent) = call.key.parent_span else {
        return Ok(());
    };
    let information = get_error_information(
        &error_info::logging_context(call.info, call.exception),
        call.traceback,
    );
    stamp_parent_error(parent, call.key);
    record_exception_on_span(
        &mut *parent.lock(),
        error_info::recorded_exception(call.exception).as_ref(),
        &error_info::error_payload(&information),
    );
    let error_code = information.error_code.as_ref().and_then(Option::as_deref);
    if error_code.unwrap_or_default().is_empty() {
        set_response_status_code(Some(parent), Some(500));
    }
    set_preprocessing_duration(Some(parent), call.request_data);
    write_failed_child(lifecycle, call, parent);
    lifecycle.emit_request_guardrails(call.request_data, Some(parent))?;
    parent
        .lock()
        .end_with_timestamp(instant_from_ns(lifecycle.now_ns()));
    Ok(())
}

// litellm.integrations.opentelemetry.OpenTelemetry.async_post_call_success_hook
pub fn async_post_call_success_hook(
    lifecycle: &LegacyLifecycle,
    model_call_details: &MetadataMap,
    parent: Option<&LiveSpan>,
) -> Result<(), ServiceCallbackError> {
    let resolved = parent::from_kwargs(model_call_details)?;
    set_preprocessing_duration(parent, model_call_details);
    let context = resolved.context.unwrap_or_default();
    guardrail::create_guardrail_span(lifecycle, model_call_details, &context)?;
    Ok(())
}

fn stamp_parent_error(span: &LiveSpan, key: &ProxyCallKey<'_>) {
    let mut target = span.lock();
    target.set_status(Status::error(""));
    set_team_attributes(&mut *target, key.team_id, key.team_alias);
}

fn write_failed_child(lifecycle: &LegacyLifecycle, call: &PostCallFailure<'_>, parent: &LiveSpan) {
    let context = Context::new().with_remote_span_context(parent.context.clone());
    let builder = lifecycle.tracer.span_builder(FAILED_REQUEST_SPAN_NAME);
    let mut span = lifecycle.tracer.build_with_context(builder, &context);
    span.safe_set_attribute(
        "exception",
        &OtelModelValue::String(error_info::exception_text(call.exception)),
    );
    set_team_attributes(&mut span, call.key.team_id, call.key.team_alias);
    span.set_status(Status::error(""));
    span.end_with_timestamp(instant_from_ns(lifecycle.now_ns()));
}
