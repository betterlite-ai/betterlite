#![cfg(test)]

use std::collections::BTreeMap;

use super::{Config, DEFAULT_ENABLED, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.cache_cost_routing)
}

fn configured(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n        {fields}\n"
    )
}

fn load(fields: &str) -> Settings {
    load_with(&configured(fields), &[]).expect("the fixture config loads")
}

#[test]
fn a_configured_deployment_converts() {
    let settings = load("cache_aware_routing: true");
    assert_eq!(
        settings.deployments,
        vec![Some(Config {
            enabled: true,
            output_tokens: 1024,
            timeout_ms: 2000,
        })]
    );
}

#[test]
fn an_explicit_output_budget_is_kept() {
    let settings = load("cache_aware_routing_output_tokens: 2048");
    assert_eq!(
        settings.deployments,
        vec![Some(Config {
            enabled: false,
            output_tokens: 2048,
            timeout_ms: 2000,
        })]
    );
}

#[test]
fn an_output_budget_below_zero_names_its_path() {
    let error = load_with(&configured("cache_aware_routing_output_tokens: -1"), &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cache_aware_routing_output_tokens"),
        "{error}"
    );
}

#[test]
fn a_zero_timeout_names_its_path() {
    let error = load_with(&configured("cache_aware_routing_timeout_ms: 0"), &[]).unwrap_err();
    assert!(
        error.to_string().contains(
            "model_list.0.litellm_params.complexity_router_config.cache_aware_routing_timeout_ms"
        ),
        "{error}"
    );
}

#[test]
fn an_environment_flag_resolves() {
    let settings = load_with(
        &configured("cache_aware_routing: os.environ/CACHE"),
        &[("CACHE", "yes")],
    )
    .expect("the fixture config loads");
    assert_eq!(
        settings.deployments[0].map(|config| config.enabled),
        Some(true)
    );
}

#[test]
fn an_empty_environment_flag_takes_the_default() {
    let settings = load_with(
        &configured("cache_aware_routing: os.environ/CACHE"),
        &[("CACHE", "")],
    )
    .expect("the fixture config loads");
    assert_eq!(
        settings.deployments[0].map(|config| config.enabled),
        Some(DEFAULT_ENABLED)
    );
}
