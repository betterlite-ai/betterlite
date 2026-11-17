use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::Context;
use opentelemetry::trace::{SpanKind, Tracer as _};
use opentelemetry_sdk::trace::SdkTracer;
use serde_json::Number;

use crate::otel::legacy_policy::span_time::{SpanTime, to_ns};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::w3c_propagation::traceparent_from_header;
use crate::otel::span_attributes::SafeSetAttribute;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::constants::attribute_keys::http::{
    RESPONSE_STATUS_CODE, ROUTE, URL_PATH,
};
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

// litellm.integrations.opentelemetry.OpenTelemetry.create_litellm_proxy_request_started_span
#[must_use]
pub fn start_proxy_request_span(
    tracer: &SdkTracer,
    start_time: SpanTime,
    now_ns: i64,
    headers: Option<&BTreeMap<String, Option<String>>>,
) -> LiveSpan {
    let instant = instant_from_ns(to_ns(start_time, now_ns));
    let parent = traceparent_from_header(headers).unwrap_or_else(Context::current);
    let builder = tracer
        .span_builder(PROXY_REQUEST_SPAN_NAME)
        .with_kind(SpanKind::Server)
        .with_start_time(instant);
    LiveSpan::from(tracer.build_with_context(builder, &parent))
}

// litellm.integrations.opentelemetry.OpenTelemetry.set_proxy_request_route_attributes
pub fn set_proxy_request_route(
    span: Option<&LiveSpan>,
    url_path: Option<&str>,
    http_route: Option<&str>,
) {
    let Some(live) = span else {
        return;
    };
    let mut target = live.lock();
    for (key, value) in [(URL_PATH, url_path), (ROUTE, http_route)] {
        if let Some(text) = value.filter(|text| !text.is_empty()) {
            target.safe_set_attribute(key, &OtelModelValue::String(text.to_owned()));
        }
    }
}

// litellm.integrations.opentelemetry.OpenTelemetry.set_response_status_code_attribute
pub fn set_response_status_code(span: Option<&LiveSpan>, status_code: Option<i64>) {
    let Some(live) = span else {
        return;
    };
    let Some(code) = status_code else {
        return;
    };
    live.lock().safe_set_attribute(
        RESPONSE_STATUS_CODE,
        &OtelModelValue::Number(Number::from(code)),
    );
}

// _to_ns returns whole nanoseconds; SystemTime carries them the same way for both signs
fn instant_from_ns(nanoseconds: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(nanoseconds.unsigned_abs());
    if nanoseconds.is_negative() {
        UNIX_EPOCH.checked_sub(magnitude).unwrap_or(UNIX_EPOCH)
    } else {
        UNIX_EPOCH.checked_add(magnitude).unwrap_or(UNIX_EPOCH)
    }
}
