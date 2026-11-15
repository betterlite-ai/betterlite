#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::deployment_clients::{DeploymentLimits, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.deployment_clients)
        .map_err(|error| error.to_string())
}

#[test]
fn client_ttl_defaults_to_one_hour_and_no_router_cap() {
    let settings = settings("model_list: []\n", &[]).unwrap();
    assert_eq!(settings.client_ttl_seconds, 3600);
    assert_eq!(settings.default_max_parallel_requests, None);
    assert!(settings.deployments.is_empty());
}

#[test]
fn router_settings_read_the_cap_and_the_ttl() {
    let settings = settings(
        "router_settings:\n  client_ttl: 60\n  default_max_parallel_requests: 7\nmodel_list: []\n",
        &[],
    )
    .unwrap();
    assert_eq!(settings.client_ttl_seconds, 60);
    assert_eq!(settings.default_max_parallel_requests, Some(7));
}

#[test]
fn deployments_keep_model_list_order() {
    let settings = settings(
        "model_list:\n  - model_name: a\n    litellm_params: {model: openai/a, max_parallel_requests: 2, rpm: 3, tpm: 4000}\n  - model_name: b\n    litellm_params: {model: openai/b}\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        settings.deployments.to_vec(),
        vec![
            DeploymentLimits {
                max_parallel_requests: Some(2),
                rpm: Some(3),
                tpm: Some(4000),
            },
            DeploymentLimits::default(),
        ]
    );
}

#[test]
fn environment_values_are_resolved_once() {
    let settings = settings(
        "router_settings:\n  client_ttl: os.environ/CLIENT_TTL\n  default_max_parallel_requests: os.environ/DEFAULT_PARALLEL\nmodel_list:\n  - model_name: a\n    litellm_params: {model: openai/a, max_parallel_requests: os.environ/PARALLEL}\n",
        &[
            ("CLIENT_TTL", "90"),
            ("DEFAULT_PARALLEL", "5"),
            ("PARALLEL", "4"),
        ],
    )
    .unwrap();
    assert_eq!(settings.client_ttl_seconds, 90);
    assert_eq!(settings.default_max_parallel_requests, Some(5));
    assert_eq!(settings.deployments[0].max_parallel_requests, Some(4));
}

#[rstest]
#[case::ttl_negative("router_settings: {client_ttl: -1}\nmodel_list: []\n")]
#[case::ttl_text("router_settings: {client_ttl: soon}\nmodel_list: []\n")]
fn a_bad_client_ttl_is_an_error_that_names_the_field(#[case] source: &str) {
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("router_settings.client_ttl"), "{error}");
}

#[test]
fn a_missing_environment_value_is_an_error() {
    let error = settings(
        "router_settings:\n  default_max_parallel_requests: os.environ/MISSING\nmodel_list: []\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("MISSING"), "{error}");
}

#[test]
fn a_text_limit_is_an_error() {
    let error = settings(
        "model_list:\n  - model_name: a\n    litellm_params: {model: openai/a, max_parallel_requests: soon}\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("max_parallel_requests"), "{error}");
}

#[test]
fn a_negative_limit_is_an_error_that_names_the_field() {
    let error = settings(
        "model_list:\n  - model_name: a\n    litellm_params: {model: openai/a, tpm: -1}\n",
        &[],
    )
    .unwrap_err();
    assert!(error.contains("model_list.0.litellm_params.tpm"), "{error}");
}
