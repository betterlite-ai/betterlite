use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_lifecycle::tracer::{
    DynamicOtel, TracerChoice, dynamic_params,
};
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;

use crate::support::{Rig, in_memory_config, simple_kwargs, with_dynamic};

struct ConfigFromParams;

impl DynamicOtel for ConfigFromParams {
    fn construct_dynamic_otel_headers(
        &self,
        _standard_callback_dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        None
    }

    fn construct_dynamic_otel_config(
        &self,
        standard_callback_dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        standard_callback_dynamic_params
            .contains_key("langfuse_public_key")
            .then(in_memory_config)
    }
}

#[test]
fn dynamic_config_is_built_from_the_dynamic_params_bucket() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(
        in_memory_config(),
        None,
        Some(std::sync::Arc::new(ConfigFromParams)),
        false,
    );
    let kwargs = with_dynamic(
        simple_kwargs("call-1"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    let params = dynamic_params(&kwargs).expect("object bucket");
    assert!(params.is_some(), "the bucket reaches the hook");
    assert!(matches!(
        lifecycle.tracer_choice(&kwargs).expect("tracer choice"),
        TracerChoice::DynamicConfig(_)
    ));
}

#[test]
fn dynamic_config_is_absent_without_dynamic_params() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(
        in_memory_config(),
        None,
        Some(std::sync::Arc::new(ConfigFromParams)),
        false,
    );
    assert!(matches!(
        lifecycle
            .tracer_choice(&simple_kwargs("call-2"))
            .expect("tracer choice"),
        TracerChoice::Global
    ));
}

#[test]
fn dynamic_config_treats_an_empty_bucket_as_unset() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(
        in_memory_config(),
        None,
        Some(std::sync::Arc::new(ConfigFromParams)),
        false,
    );
    let kwargs = with_dynamic(simple_kwargs("call-3"), serde_json::json!({}));
    assert!(
        dynamic_params(&kwargs).expect("empty bucket").is_none(),
        "an empty object is unset"
    );
    assert!(matches!(
        lifecycle.tracer_choice(&kwargs).expect("tracer choice"),
        TracerChoice::Global
    ));
}

#[test]
fn dynamic_config_treats_null_as_unset() {
    let kwargs = with_dynamic(simple_kwargs("call-4"), serde_json::json!(null));
    assert!(dynamic_params(&kwargs).expect("null bucket").is_none());
}

#[test]
fn dynamic_config_rejects_a_non_object_bucket() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(
        in_memory_config(),
        None,
        Some(std::sync::Arc::new(ConfigFromParams)),
        false,
    );
    let mut kwargs = simple_kwargs("call-5");
    kwargs.insert(
        "standard_callback_dynamic_params".to_owned(),
        OtelModelValue::String("pk".to_owned()),
    );
    assert!(dynamic_params(&kwargs).is_err());
    assert!(lifecycle.tracer_choice(&kwargs).is_err());
}
