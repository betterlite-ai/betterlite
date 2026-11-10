use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

mod tests;

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

// LiteLLM asks `if value:` before every optional write; the rule matches B3861's is_truthy
#[must_use]
pub fn truthiness(value: &OtelModelValue) -> bool {
    match value {
        OtelModelValue::Null(()) => false,
        OtelModelValue::Bool(flag) => *flag,
        OtelModelValue::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        OtelModelValue::String(text) => !text.is_empty(),
        OtelModelValue::Array(items) => !items.is_empty(),
        OtelModelValue::Object(fields) => !fields.is_empty(),
    }
}
