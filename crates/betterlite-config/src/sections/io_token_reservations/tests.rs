#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::io_token_reservations::{DeploymentIoLimits, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.io_token_reservations)
        .map_err(|error| error.to_string())
}

fn deployment(fields: &str) -> String {
    format!("model_list:\n  - model_name: probe\n{fields}")
}

fn one_deployment(source: &str) -> DeploymentIoLimits {
    let deployments = settings(source, &[]).unwrap().deployments;
    assert_eq!(deployments.len(), 1);
    deployments[0]
}

#[test]
fn deployment_level_wins_over_litellm_params_and_model_info() {
    let source = deployment(
        "    itpm: 1\n    otpm: 2\n    litellm_params: {itpm: 3, otpm: 4}\n    model_info: {itpm: 5, otpm: 6}\n",
    );
    assert_eq!(
        one_deployment(&source),
        DeploymentIoLimits {
            itpm: Some(1),
            otpm: Some(2),
        }
    );
}

#[test]
fn litellm_params_wins_when_the_deployment_level_is_absent() {
    let source =
        deployment("    litellm_params: {itpm: 3, otpm: 4}\n    model_info: {itpm: 5, otpm: 6}\n");
    assert_eq!(
        one_deployment(&source),
        DeploymentIoLimits {
            itpm: Some(3),
            otpm: Some(4),
        }
    );
}

#[test]
fn model_info_is_the_last_fallback() {
    let source = deployment("    model_info: {itpm: 5, otpm: 6}\n");
    assert_eq!(
        one_deployment(&source),
        DeploymentIoLimits {
            itpm: Some(5),
            otpm: Some(6),
        }
    );
}

#[test]
fn a_text_value_is_trimmed() {
    let source = deployment("    itpm: \" 42 \"\n");
    assert_eq!(one_deployment(&source).itpm, Some(42));
}

#[test]
fn an_explicit_zero_is_kept() {
    let source = deployment("    itpm: 0\n    litellm_params: {itpm: 3}\n");
    assert_eq!(one_deployment(&source).itpm, Some(0));
}

#[test]
fn an_explicit_null_falls_through() {
    let source = deployment("    itpm: null\n    litellm_params: {itpm: 3}\n");
    assert_eq!(one_deployment(&source).itpm, Some(3));
}

#[test]
fn an_environ_reference_resolves() {
    let source = deployment("    litellm_params: {itpm: os.environ/ITPM_LIMIT}\n");
    let limits = settings(&source, &[("ITPM_LIMIT", "250")])
        .unwrap()
        .deployments[0];
    assert_eq!(limits.itpm, Some(250));
}

#[test]
fn an_empty_env_value_counts_as_unset() {
    let source = deployment(
        "    litellm_params: {itpm: os.environ/ITPM_LIMIT}\n    model_info: {itpm: 7}\n",
    );
    let limits = settings(&source, &[("ITPM_LIMIT", "")])
        .unwrap()
        .deployments[0];
    assert_eq!(limits.itpm, Some(7));
}

#[test]
fn a_lower_level_is_not_resolved_once_a_higher_one_is_set() {
    let source = deployment("    itpm: 10\n    litellm_params: {itpm: os.environ/UNSET_ITPM}\n");
    assert_eq!(one_deployment(&source).itpm, Some(10));
}

#[test]
fn a_bad_env_value_names_the_variable() {
    let source = deployment("    itpm: os.environ/ITPM_LIMIT\n");
    let error = settings(&source, &[("ITPM_LIMIT", "many")]).unwrap_err();
    assert!(error.contains("ITPM_LIMIT"), "{error}");
}

#[rstest]
#[case::boolean("true")]
#[case::float("1.5")]
#[case::text("many")]
#[case::empty_string("\"\"")]
fn a_bad_scalar_names_the_key(#[case] value: &str) {
    let source = deployment(&format!("    itpm: {value}\n"));
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("model_list.0.itpm"), "{error}");
}

#[test]
fn deployments_stay_index_aligned() {
    let source = "model_list:\n  - model_name: first\n    itpm: 1\n  - model_name: second\n    otpm: 2\n  - model_name: third\n    litellm_params: {itpm: 3, otpm: 4}\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved.deployments.as_ref(),
        [
            DeploymentIoLimits {
                itpm: Some(1),
                otpm: None,
            },
            DeploymentIoLimits {
                itpm: None,
                otpm: Some(2),
            },
            DeploymentIoLimits {
                itpm: Some(3),
                otpm: Some(4),
            },
        ]
        .as_slice()
    );
}

#[test]
fn a_deployment_without_limits_keeps_none() {
    assert_eq!(
        one_deployment(&deployment("")),
        DeploymentIoLimits::default()
    );
}

#[test]
fn an_empty_model_list_has_no_deployments() {
    assert!(
        settings("model_list: []", &[])
            .unwrap()
            .deployments
            .is_empty()
    );
}
