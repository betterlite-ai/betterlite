#![cfg(test)]
use super::loader::{ConfigError, ConfigLoader};
use super::types::RoutingStrategy;
use secrecy::ExposeSecret;
use std::collections::BTreeMap;

mod provider_base_config;

fn map_loader(values: BTreeMap<String, String>) -> ConfigLoader<BTreeMap<String, String>> {
    ConfigLoader { lookup: values }
}
#[test]
fn loads_repeated_deployments_in_file_order() {
    let config = map_loader(BTreeMap::new())
        .load(
            "model_list:\n  - model_name: same\n    litellm_params:\n      model: first\n  - model_name: same\n    litellm_params:\n      model: second\n",
        )
        .unwrap();

    assert_eq!(config.model_list.len(), 2);
    assert_eq!(config.model_list[0].model_name, "same");
    assert_eq!(
        config.model_list[0].litellm_params.model.as_deref(),
        Some("first")
    );
    assert_eq!(
        config.model_list[1].litellm_params.model.as_deref(),
        Some("second")
    );
}
#[test]
fn loads_general_router_and_model_info() {
    let mut values = BTreeMap::new();
    values.insert("MASTER".to_owned(), "master-secret".to_owned());
    let config = map_loader(values)
        .load(
            "model_list:\n  - model_name: same\n    litellm_params:\n      model: first\n    model_info:\n      id: model-id\ngeneral_settings:\n  master_key: os.environ/MASTER\nrouter_settings:\n  routing_strategy: simple-shuffle\n  num_retries: 2\n  timeout: 5\n  allowed_fails: 3\n  cooldown_time: 10\nlitellm_settings:\n  num_retries: 4\n  request_timeout: 6\n  drop_params: true\n",
        )
        .unwrap();

    assert_eq!(
        config.model_list[0].model_info.id.as_deref(),
        Some("model-id")
    );
    assert!(config.general_settings.master_key.is_some());
    assert_eq!(
        config.router_settings.routing_strategy,
        Some(RoutingStrategy::SimpleShuffle)
    );
    assert_eq!(config.router_settings.num_retries, Some(2));
    assert_eq!(config.litellm_settings.num_retries, Some(4));
    map_loader(BTreeMap::new())
        .load("router_settings:\n  routing_strategy: unsupported\n")
        .unwrap_err();
    let environment = map_loader(BTreeMap::new())
        .load("environment_variables:\n  REGION: eu\n")
        .unwrap();
    assert!(environment.environment_variables.contains_key("REGION"));
}
#[test]
fn resolves_environment_values_from_map() {
    let mut values = BTreeMap::new();
    values.insert("OPENAI_KEY".to_owned(), "secret-value".to_owned());
    values.insert("REQUEST_TIMEOUT".to_owned(), "12.5".to_owned());
    let config = map_loader(values)
        .load(
            "model_list:\n  - model_name: x\n    litellm_params:\n      model: openai/x\n      api_key: os.environ/OPENAI_KEY\n      timeout: os.environ/REQUEST_TIMEOUT\n",
        )
        .unwrap();

    assert_eq!(
        config.model_list[0]
            .litellm_params
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("secret-value")
    );
    assert_eq!(config.model_list[0].litellm_params.timeout, Some(12.5_f64));
}
#[test]
fn debug_does_not_expose_secrets() {
    let mut values = BTreeMap::new();
    values.insert("API_KEY".to_owned(), "api-secret".to_owned());
    values.insert("MASTER_KEY".to_owned(), "master-secret".to_owned());
    values.insert("REDIS_PASSWORD".to_owned(), "redis-secret".to_owned());
    let config = map_loader(values)
        .load(
            "model_list:\n  - model_name: x\n    litellm_params:\n      model: openai/x\n      api_key: os.environ/API_KEY\ngeneral_settings:\n  master_key: os.environ/MASTER_KEY\nenvironment_variables:\n  REDIS_PASSWORD: os.environ/REDIS_PASSWORD\n",
        )
        .unwrap();
    let debug = format!("{config:?}");

    assert!(!debug.contains("api-secret"));
    assert!(!debug.contains("master-secret"));
    assert!(!debug.contains("redis-secret"));
}

#[test]
fn rejects_boolean_in_numeric_field_as_invalid_scalar() {
    assert!(
        matches!(map_loader(BTreeMap::new()).load("model_list:\n  - model_name: x\n    litellm_params:\n      timeout: true\n"), Err(ConfigError::InvalidScalar { path, value: true }) if path == "model_list.0.litellm_params.timeout")
    );
}

mod bedrock_request_policy_config;
