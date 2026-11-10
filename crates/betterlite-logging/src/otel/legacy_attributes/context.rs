use opentelemetry::trace::Span;

use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;

pub struct AttributeContext<'request> {
    pub callback_name: Option<&'request str>,
    pub config: &'request OpenTelemetryConfig,
    pub capture_mode: CaptureMode,
}

// clippy stops at five parameters, so the per-call maps travel together
pub struct InferenceCall<'request> {
    pub kwargs: &'request MetadataMap,
    pub optional_params: &'request MetadataMap,
    pub litellm_params: &'request MetadataMap,
    pub payload: &'request MetadataMap,
    pub response: Option<&'request OtelModelValue>,
}

pub(super) fn write_present<S>(span: &mut S, key: &str, source: Option<&OtelModelValue>)
where
    S: Span,
{
    if let Some(value) = source.filter(|value| is_truthy(value)) {
        span.safe_set_attribute(key, value);
    }
}
