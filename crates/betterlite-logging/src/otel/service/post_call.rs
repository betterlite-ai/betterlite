use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status, TraceContextExt as _, Tracer as _};

use crate::logging_metadata::error_info::get_error_information;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_lifecycle::context::{instant_from_ns, standard_logging_object};
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
    set_preprocessing_duration(parent, model_call_details);
    let Some(request_data) = guardrail_request_data(model_call_details) else {
        return Ok(());
    };
    let resolved = parent::from_kwargs(model_call_details)?;
    let parent_span = guardrail_parent(lifecycle, resolved.context.as_ref());
    lifecycle.emit_request_guardrails(&request_data, parent_span.as_ref())?;
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

// _create_guardrail_span reads the entries from standard_logging_object and the markers from litellm_params.metadata
fn guardrail_request_data(model_call_details: &MetadataMap) -> Option<MetadataMap> {
    let payload = standard_logging_object(model_call_details)?;
    let information = payload.get("guardrail_information")?;
    let logged = payload.get("metadata").and_then(OtelModelValue::as_object);
    let mut metadata = request_metadata(model_call_details)
        .and_then(OtelModelValue::as_object)
        .cloned()
        .unwrap_or_default();
    for key in ["user_api_key_team_id", "user_api_key_team_alias"] {
        if metadata.contains_key(key) {
            continue;
        }
        if let Some(value) = logged.and_then(|fields| fields.get(key)) {
            metadata.insert(key.to_owned(), value.clone());
        }
    }
    metadata.insert(
        "standard_logging_guardrail_information".to_owned(),
        information.clone(),
    );
    Some(MetadataMap::from([(
        "metadata".to_owned(),
        OtelModelValue::Object(metadata),
    )]))
}

// the proxy puts its own metadata under litellm_params.metadata, or litellm_metadata on the newer routes
fn request_metadata(model_call_details: &MetadataMap) -> Option<&OtelModelValue> {
    let params = model_call_details
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)?;
    params
        .get("metadata")
        .or_else(|| params.get("litellm_metadata"))
}

// _get_span_context returns a context holding the request's parent span; only a live span can parent a child here
fn guardrail_parent(lifecycle: &LegacyLifecycle, context: Option<&Context>) -> Option<LiveSpan> {
    let context = context?;
    let span_context = context.span().span_context().clone();
    span_context
        .is_valid()
        .then(|| lifecycle.live.get(span_context.span_id()))
        .flatten()
}
