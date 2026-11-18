use std::collections::BTreeMap;
use std::sync::Arc;

use crate::otel::legacy_lifecycle::lifecycle::{LegacyLifecycle, LegacyLifecycleOptions};
use crate::otel::legacy_lifecycle::tracer::DynamicOtel;
use crate::otel::legacy_policy::capture::{CapturePolicy, EVENT_ONLY};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::presets::weave::weave_dynamic_headers;
use crate::otel_provider_assembly::legacy_otel_init::LegacyOtel;

pub const CALLBACK_NAME: &str = "weave_otel";

pub struct WeaveDynamicOtel;

impl DynamicOtel for WeaveDynamicOtel {
    fn construct_dynamic_otel_headers(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        let headers = weave_dynamic_headers(&dynamic_strings(dynamic_params));
        (!headers.is_empty()).then_some(headers)
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        None
    }
}

// the two params WeaveLogger.construct_dynamic_otel_headers reads out of the dynamic bucket
fn dynamic_strings(dynamic_params: &MetadataMap) -> BTreeMap<String, String> {
    ["wandb_api_key", "weave_project_id"]
        .iter()
        .filter_map(|name| {
            let value = dynamic_params.get(*name).and_then(OtelModelValue::as_str)?;
            Some(((*name).to_owned(), value.to_owned()))
        })
        .collect()
}

// WeaveOtelLogger's three overrides on the shared lifecycle:
// https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/weave/weave_otel.py#L229-L331
#[must_use]
pub fn weave_lifecycle(otel: LegacyOtel, mut options: LegacyLifecycleOptions) -> LegacyLifecycle {
    // _start_primary_span always creates a child span, even when a parent span is provided
    options.use_request_span = true;
    options.dynamic = Arc::new(WeaveDynamicOtel);
    let mut lifecycle = LegacyLifecycle::from_otel(otel, options);
    // _maybe_log_raw_request is overridden to do nothing; an event-only capture policy is the
    // lifecycle's own way to skip that span, and a vendor callback ignores the mode for its attributes
    lifecycle.capture_policy = event_only_policy(&lifecycle.config);
    lifecycle
}

fn event_only_policy(config: &OpenTelemetryConfig) -> CapturePolicy {
    let mut narrowed = config.clone();
    narrowed.capture_message_content = Some(EVENT_ONLY.to_owned());
    CapturePolicy::from_init_state(&narrowed, &BTreeMap::new())
}

mod tests;
