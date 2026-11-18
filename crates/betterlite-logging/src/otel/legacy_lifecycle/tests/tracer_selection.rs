use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_lifecycle::tracer::{
    BaseDynamicOtel, DynamicOtel, TracerChoice,
};
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;

use crate::support::{ProbeDynamic, Rig, in_memory_config, simple_kwargs, with_dynamic};

fn choose(
    lifecycle: &betterlite_logging::otel::legacy_lifecycle::lifecycle::LegacyLifecycle,
    kwargs: &MetadataMap,
) -> TracerChoice {
    lifecycle.tracer_choice(kwargs).expect("tracer choice")
}

#[test]
fn tracer_selection_prefers_the_dynamic_config_over_headers() {
    let rig = Rig::new();
    let dynamic: Arc<dyn DynamicOtel> = Arc::new(ProbeDynamic {
        config: Some(in_memory_config()),
        headers: Some(BTreeMap::from([("x-key".to_owned(), "pk".to_owned())])),
    });
    let lifecycle = rig.with_options(in_memory_config(), None, Some(dynamic), false);
    let kwargs = with_dynamic(
        simple_kwargs("call-1"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    assert!(matches!(
        choose(&lifecycle, &kwargs),
        TracerChoice::DynamicConfig(_)
    ));
}

#[test]
fn tracer_selection_uses_dynamic_headers_when_the_config_is_absent() {
    let rig = Rig::new();
    let dynamic: Arc<dyn DynamicOtel> = Arc::new(ProbeDynamic {
        config: None,
        headers: Some(BTreeMap::from([("x-key".to_owned(), "pk".to_owned())])),
    });
    let lifecycle = rig.with_options(in_memory_config(), None, Some(dynamic), false);
    let kwargs = with_dynamic(
        simple_kwargs("call-2"),
        serde_json::json!({"langfuse_public_key": "pk"}),
    );
    assert!(matches!(
        choose(&lifecycle, &kwargs),
        TracerChoice::DynamicHeaders(_)
    ));
}

#[test]
fn tracer_selection_ignores_dynamic_hooks_without_dynamic_params() {
    let rig = Rig::new();
    let dynamic: Arc<dyn DynamicOtel> = Arc::new(ProbeDynamic {
        config: Some(in_memory_config()),
        headers: Some(BTreeMap::from([("x-key".to_owned(), "pk".to_owned())])),
    });
    let lifecycle = rig.with_options(in_memory_config(), None, Some(dynamic), false);
    assert!(matches!(
        choose(&lifecycle, &simple_kwargs("call-3")),
        TracerChoice::Global
    ));
}

#[test]
fn tracer_selection_ignores_an_empty_dynamic_bucket() {
    let rig = Rig::new();
    let dynamic: Arc<dyn DynamicOtel> = Arc::new(ProbeDynamic {
        config: Some(in_memory_config()),
        headers: None,
    });
    let lifecycle = rig.with_options(in_memory_config(), None, Some(dynamic), false);
    let kwargs = with_dynamic(simple_kwargs("call-4"), serde_json::json!({}));
    assert!(matches!(choose(&lifecycle, &kwargs), TracerChoice::Global));
}

#[test]
fn tracer_selection_uses_langfuse_env_headers_for_the_langfuse_callback() {
    let rig = Rig::new();
    let config = OpenTelemetryConfig {
        headers: Some("x-langfuse-public-key=pk".to_owned()),
        ..in_memory_config()
    };
    let lifecycle = rig.with_options(config, Some("langfuse_otel"), None, false);
    let choice = choose(&lifecycle, &simple_kwargs("call-5"));
    let TracerChoice::EnvHeaders(headers) = choice else {
        panic!("expected the env-var header branch");
    };
    assert_eq!(
        headers.get("x-langfuse-public-key").map(String::as_str),
        Some("pk")
    );
}

#[test]
fn tracer_selection_falls_back_to_the_global_tracer() {
    let rig = Rig::new();
    let lifecycle = rig.with_options(in_memory_config(), Some("langfuse_otel"), None, false);
    assert!(matches!(
        choose(&lifecycle, &simple_kwargs("call-6")),
        TracerChoice::Global
    ));
}

#[test]
fn dynamic_base_hooks_return_no_override() {
    let dynamic = BaseDynamicOtel;
    let params = MetadataMap::new();
    assert!(dynamic.construct_dynamic_otel_headers(&params).is_none());
    assert!(dynamic.construct_dynamic_otel_config(&params).is_none());
}
