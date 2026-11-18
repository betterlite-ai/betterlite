use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_lifecycle::tracer::{DynamicOtel, TracerChoice};
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;

use crate::support::{ProbeDynamic, Rig, in_memory_config, simple_kwargs, with_dynamic};

struct HeadersFromParams;

impl DynamicOtel for HeadersFromParams {
    fn construct_dynamic_otel_headers(
        &self,
        dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        dynamic_params
            .get("langfuse_public_key")
            .and_then(OtelModelValue::as_str)
            .map(|key| BTreeMap::from([("x-langfuse-public-key".to_owned(), key.to_owned())]))
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        None
    }
}

#[test]
fn dynamic_headers_are_built_from_the_dynamic_params_bucket() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(
        in_memory_config(),
        None,
        Some(Arc::new(HeadersFromParams)),
        false,
    );
    let kwargs = with_dynamic(
        simple_kwargs("call-1"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    let TracerChoice::DynamicHeaders(headers) =
        lifecycle.tracer_choice(&kwargs).expect("tracer choice")
    else {
        panic!("expected dynamic headers");
    };
    assert_eq!(
        headers.get("x-langfuse-public-key").map(String::as_str),
        Some("pk")
    );
}

#[test]
fn dynamic_headers_empty_map_falls_back_to_the_global_tracer() {
    let rig = Rig::new();
    let dynamic: Arc<dyn DynamicOtel> = Arc::new(ProbeDynamic {
        config: None,
        headers: Some(BTreeMap::new()),
    });
    let lifecycle = rig.with_options(in_memory_config(), None, Some(dynamic), false);
    let kwargs = with_dynamic(
        simple_kwargs("call-2"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    assert!(matches!(
        lifecycle.tracer_choice(&kwargs).expect("tracer choice"),
        TracerChoice::Global
    ));
}

#[test]
fn dynamic_headers_win_over_the_langfuse_environment_headers() {
    let rig = Rig::new();
    let config = OpenTelemetryConfig {
        headers: Some("x-langfuse-public-key=env-pk".to_owned()),
        ..in_memory_config()
    };
    let lifecycle = rig.with_options(
        config,
        Some("langfuse_otel"),
        Some(Arc::new(HeadersFromParams)),
        false,
    );
    let kwargs = with_dynamic(
        simple_kwargs("call-3"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    let TracerChoice::DynamicHeaders(headers) =
        lifecycle.tracer_choice(&kwargs).expect("tracer choice")
    else {
        panic!("expected dynamic headers");
    };
    assert_eq!(
        headers.get("x-langfuse-public-key").map(String::as_str),
        Some("pk")
    );
}

#[test]
fn dynamic_headers_env_branch_is_reserved_for_the_langfuse_callback() {
    let rig = Rig::new();
    let config = OpenTelemetryConfig {
        headers: Some("x-langfuse-public-key=pk".to_owned()),
        ..in_memory_config()
    };
    let lifecycle = rig.with_options(config, Some("arize"), None, false);
    assert!(matches!(
        lifecycle
            .tracer_choice(&simple_kwargs("call-4"))
            .expect("tracer choice"),
        TracerChoice::Global
    ));
}
