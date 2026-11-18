use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::trace::{Span as _, SpanKind, Status, TraceContextExt as _, Tracer as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::Span as SdkSpan;

use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_lifecycle::lifecycle::LegacyLifecycle;
use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::model::db_endpoint::db_span_attributes;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::constants::attribute_keys::{error, litellm};

use super::timing;

pub struct ServiceSpanCall<'call> {
    pub payload: &'call ServiceLoggerPayload,
    pub parent: Option<&'call LiveSpan>,
    pub start_time: SpanTime,
    pub end_time: SpanTime,
    pub event_metadata: Option<&'call MetadataMap>,
}

// litellm.integrations.opentelemetry.OpenTelemetry.async_service_success_hook
pub fn async_service_success_hook(
    lifecycle: &LegacyLifecycle,
    call: &ServiceSpanCall<'_>,
    environment: &dyn EnvironmentLookup,
) {
    let Some(parent) = call.parent else {
        return;
    };
    let start_ns = to_ns(call.start_time, lifecycle.now_ns());
    let end_ns = to_ns(call.end_time, lifecycle.now_ns());
    let span = start_service_span(lifecycle, call, parent, start_ns, environment);
    write_event_metadata(&mut *span.lock(), call.event_metadata, true);
    span.lock().set_status(Status::Ok);
    span.lock().end_with_timestamp(instant_from_ns(end_ns));
}

// litellm.integrations.opentelemetry.OpenTelemetry.async_service_failure_hook
pub fn async_service_failure_hook(
    lifecycle: &LegacyLifecycle,
    call: &ServiceSpanCall<'_>,
    error_text: Option<&str>,
    environment: &dyn EnvironmentLookup,
) {
    let Some(parent) = call.parent else {
        return;
    };
    let start_ns = timing::whole_second_ns(call.start_time, lifecycle.now_ns());
    let end_ns = timing::whole_second_ns(call.end_time, lifecycle.now_ns());
    let span = start_service_span(lifecycle, call, parent, start_ns, environment);
    if let Some(text) = error_text.filter(|text| !text.is_empty()) {
        span.lock().safe_set_attribute(
            error::MESSAGE_LEGACY,
            &OtelModelValue::String(text.to_owned()),
        );
    }
    write_event_metadata(&mut *span.lock(), call.event_metadata, false);
    span.lock().set_status(Status::error(""));
    span.lock().end_with_timestamp(instant_from_ns(end_ns));
}

// litellm.integrations.opentelemetry.OpenTelemetry._start_service_span
fn start_service_span(
    lifecycle: &LegacyLifecycle,
    call: &ServiceSpanCall<'_>,
    parent: &LiveSpan,
    start_ns: i64,
    environment: &dyn EnvironmentLookup,
) -> LiveSpan {
    let data = ServiceSpanData::from_payload(call.payload, &MetadataMap::new());
    let attributes = db_span_attributes(
        &data.service_name,
        Some(call.payload.call_type.as_str()),
        environment,
    );
    let kind = if attributes.is_empty() {
        SpanKind::Internal
    } else {
        SpanKind::Client
    };
    let context = Context::new().with_remote_span_context(parent.context.clone());
    let builder = lifecycle
        .tracer
        .span_builder(data.service_name.clone())
        .with_kind(kind)
        .with_start_time(instant_from_ns(start_ns));
    let span = LiveSpan::from(lifecycle.tracer.build_with_context(builder, &context));
    write_service_attributes(
        &mut *span.lock(),
        call.payload,
        &data.service_name,
        attributes,
    );
    span
}

fn write_service_attributes(
    span: &mut SdkSpan,
    payload: &ServiceLoggerPayload,
    service: &str,
    attributes: Vec<KeyValue>,
) {
    for attribute in attributes {
        span.set_attribute(attribute);
    }
    if let Some(caller) = payload.caller.as_deref() {
        span.safe_set_attribute(
            litellm::SERVICE_CALLER,
            &OtelModelValue::String(caller.to_owned()),
        );
    }
    span.safe_set_attribute(
        "call_type",
        &OtelModelValue::String(payload.call_type.clone()),
    );
    span.safe_set_attribute("service", &OtelModelValue::String(service.to_owned()));
}

// the success hook turns a null metadata value into the text "None"; the failure hook leaves it empty
fn write_event_metadata(span: &mut SdkSpan, metadata: Option<&MetadataMap>, none_as_text: bool) {
    let Some(fields) = metadata.filter(|fields| !fields.is_empty()) else {
        return;
    };
    for (key, value) in fields {
        if none_as_text && matches!(value, OtelModelValue::Null(())) {
            span.safe_set_attribute(key, &OtelModelValue::String("None".to_owned()));
            continue;
        }
        span.safe_set_attribute(key, value);
    }
}
