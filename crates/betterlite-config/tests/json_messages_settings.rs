#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    DecodeLimits, JSONProviderRegistry, decode_json_provider_registry,
};
use rstest::rstest;

const MESSAGES_ENDPOINT: &str = "/v1/messages";

fn registry() -> JSONProviderRegistry {
    // the settings come from the loader's explicit lookup, never from the ambient environment
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .expect("an empty config loads");
    assert!(loaded.model_list.is_empty());
    JSONProviderRegistry::load()
}

#[test]
fn native_messages_descriptors_declare_the_messages_endpoint() {
    let registry = registry();
    let mut native_messages: Vec<String> = registry
        .providers
        .iter()
        .filter(|(_, descriptor)| {
            descriptor
                .config
                .supported_endpoints
                .iter()
                .any(|endpoint| endpoint == MESSAGES_ENDPOINT)
        })
        .map(|(provider, _)| provider.to_string())
        .collect();
    native_messages.sort();
    assert_eq!(
        native_messages,
        vec![
            "cortecs".to_owned(),
            "meta".to_owned(),
            "prism".to_owned(),
            "sail".to_owned()
        ]
    );
}

#[rstest]
#[case::cortecs(ProviderId::Cortecs, "cortecs/example-model")]
#[case::meta(ProviderId::Meta, "meta/example-model")]
#[case::prism(ProviderId::Prism, "prism/example-model")]
#[case::sail(ProviderId::Sail, "sail/example-model")]
fn deployment_cache_control_ttl_defaults_false(#[case] provider: ProviderId, #[case] model: &str) {
    let registry = registry();
    let descriptor = registry
        .providers
        .get(&provider)
        .expect("a pinned provider");
    assert!(
        descriptor
            .config
            .supported_endpoints
            .iter()
            .any(|endpoint| endpoint == MESSAGES_ENDPOINT)
    );
    assert_eq!(descriptor.config.constraints.cache_control_ttl, None);
    let config = format!(
        "model_list:\n  - model_name: messages\n    litellm_params:\n      model: {model}\n"
    );
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&config)
    .expect("a deployment on a native messages provider loads");
    assert_eq!(
        loaded
            .model_list
            .first()
            .expect("one deployment")
            .litellm_params
            .model
            .as_deref(),
        Some(model)
    );
}

#[rstest]
#[case::explicit_true(r#"{"cache_control_ttl":true}"#, Some(true))]
#[case::explicit_false(r#"{"cache_control_ttl":false}"#, Some(false))]
#[case::absent("{}", None)]
fn descriptor_cache_control_ttl_is_explicit_when_set(
    #[case] constraints: &str,
    #[case] ttl: Option<bool>,
) {
    let source = format!(
        r#"{{"cortecs":{{"base_url":"https://api.cortecs.ai/v1","api_key_env":"CORTECS_API_KEY","supported_endpoints":["{MESSAGES_ENDPOINT}"],"constraints":{constraints}}}}}"#
    );
    let decoded = decode_json_provider_registry(source.as_bytes(), DecodeLimits::default())
        .expect("the descriptor decodes");
    let descriptor = decoded
        .providers
        .get(&ProviderId::Cortecs)
        .expect("the cortecs descriptor");
    assert_eq!(descriptor.config.constraints.cache_control_ttl, ttl);
    assert!(
        descriptor
            .config
            .supported_endpoints
            .iter()
            .any(|endpoint| endpoint == MESSAGES_ENDPOINT)
    );
}
