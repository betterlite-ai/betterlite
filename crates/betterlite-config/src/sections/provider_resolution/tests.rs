#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

fn load(source: &str, environment: &BTreeMap<String, String>) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .map(|config| config.sections.provider_resolution)
}

#[rstest]
#[case::a_declared_provider_is_read(
    "model_list:\n  - model_name: g\n    litellm_params:\n      model: gpt-4o\n      custom_llm_provider: openai\n",
    &["openai"]
)]
#[case::one_value_per_deployment_keeps_its_position(
    "model_list:\n  - model_name: a\n    litellm_params:\n      model: m\n      custom_llm_provider: azure\n  - model_name: b\n    litellm_params:\n      model: m\n  - model_name: c\n    litellm_params:\n      model: m\n      custom_llm_provider: groq\n",
    &["azure", "", "groq"]
)]
#[case::a_deployment_without_params_is_absent(
    "model_list:\n  - model_name: a\n",
    &[""]
)]
#[case::an_explicit_empty_value_is_kept(
    "model_list:\n  - model_name: a\n    litellm_params:\n      model: m\n      custom_llm_provider: ''\n",
    &[""]
)]
fn provider_resolution_tests_behavior(#[case] source: &str, #[case] expected: &[&str]) {
    let settings = load(source, &BTreeMap::new()).unwrap();
    let observed: Vec<&str> = settings
        .deployments
        .iter()
        .map(|value| value.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(observed, expected);
}

#[test]
fn an_environment_reference_is_resolved() {
    let environment = BTreeMap::from([("MODEL_PROVIDER".to_owned(), "mistral".to_owned())]);
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: m\n      custom_llm_provider: os.environ/MODEL_PROVIDER\n";
    let settings = load(source, &environment).unwrap();
    assert_eq!(settings.deployments, vec![Some(String::from("mistral"))]);
}

#[test]
fn a_missing_environment_reference_names_the_path() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: m\n      custom_llm_provider: os.environ/ABSENT_PROVIDER\n";
    let error = load(source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        &error,
        ConfigError::MissingEnvironment { variable, .. } if variable == "ABSENT_PROVIDER"
    ));
    assert!(
        error
            .to_string()
            .contains("model_list.0.litellm_params.custom_llm_provider"),
        "{error}"
    );
}

#[test]
fn an_unrelated_unknown_key_is_still_unsupported() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: m\n      not_a_litellm_key: 1\n";
    let error = load(source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { .. }), "{error}");
}
