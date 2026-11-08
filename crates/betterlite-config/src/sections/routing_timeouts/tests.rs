#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::routing_timeouts::{
    DeploymentTimeouts, GlobalTimeout, RouterTimeouts, Settings,
};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.routing_timeouts)
        .map_err(|error| error.to_string())
}

fn model_list(params: &str) -> String {
    format!(
        "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/gpt-4o\n{params}"
    )
}

#[rstest]
#[case::absent_defaults("router_settings: {}", &[], (6000.0_f64, false))]
#[case::environment_request_timeout("router_settings: {}", &[("REQUEST_TIMEOUT", "12.5")], (12.5_f64, true))]
#[case::empty_environment_is_unset("router_settings: {}", &[("REQUEST_TIMEOUT", "")], (6000.0_f64, false))]
#[case::configured_6000_is_explicit("litellm_settings:\n  request_timeout: 6000", &[], (6000.0_f64, true))]
#[case::configured_value_beats_environment(
    "litellm_settings:\n  request_timeout: 7",
    &[("REQUEST_TIMEOUT", "9")],
    (7.0_f64, true)
)]
#[case::environment_reference("litellm_settings:\n  request_timeout: os.environ/GLOBAL_TIMEOUT", &[("GLOBAL_TIMEOUT", "30")], (30.0_f64, true))]
fn global_timeout(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: (f64, bool),
) {
    let resolved = settings(source, variables).unwrap();
    assert_eq!(
        resolved.global,
        GlobalTimeout {
            seconds: expected.0,
            explicitly_set: expected.1,
        }
    );
    assert_eq!(resolved.router, RouterTimeouts::default());
    assert!(resolved.deployments.is_empty());
}

#[test]
fn router_timeouts_resolve_keys_and_environment() {
    let source = "router_settings:\n  timeout: 7\n  stream_timeout: os.environ/STREAM_TIMEOUT\n  default_litellm_params:\n    timeout: 5\n    stream_timeout: 4";
    let resolved = settings(source, &[("STREAM_TIMEOUT", "3")]).unwrap();
    assert_eq!(
        resolved.router,
        RouterTimeouts {
            timeout: Some(7.0_f64),
            stream_timeout: Some(3.0_f64),
            default_timeout: Some(5.0_f64),
            default_stream_timeout: Some(4.0_f64),
        }
    );
}

#[test]
fn deployment_timeouts_stay_aligned_with_model_list() {
    let first = "      timeout: 11\n      stream_timeout: 12\n      request_timeout: 13\n";
    let second = "      timeout: 21\n";
    let source = format!(
        "model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/gpt-4o\n{first}  - model_name: h\n    litellm_params:\n      model: openai/gpt-4o\n{second}"
    );
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![
            DeploymentTimeouts {
                timeout: Some(11.0_f64),
                stream_timeout: Some(12.0_f64),
                request_timeout: Some(13.0_f64),
            },
            DeploymentTimeouts {
                timeout: Some(21.0_f64),
                stream_timeout: None,
                request_timeout: None,
            },
        ]
    );
}

#[test]
fn deployment_environment_reference_resolves() {
    let source = model_list("      timeout: os.environ/DEPLOYMENT_TIMEOUT\n");
    let resolved = settings(&source, &[("DEPLOYMENT_TIMEOUT", "0.5")]).unwrap();
    assert_eq!(resolved.deployments[0].timeout, Some(0.5_f64));
}

#[test]
fn missing_environment_names_variable_and_path() {
    let source = model_list("      stream_timeout: os.environ/UNSET_TIMEOUT\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("UNSET_TIMEOUT"));
    assert!(error.contains("model_list.0.litellm_params.stream_timeout"));
}

#[test]
fn boolean_timeout_is_a_field_error() {
    let source = "router_settings:\n  timeout: true";
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("router_settings.timeout"));
}
