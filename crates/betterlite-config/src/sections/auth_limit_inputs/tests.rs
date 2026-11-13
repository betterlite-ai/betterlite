#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::{DeploymentLimitDefaults, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn settings(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.auth_limit_inputs)
}

#[derive(Debug, PartialEq)]
enum Scalar<'json> {
    Integer(i64),
    Float(f64),
    Text(&'json str),
    Other,
}

fn scalar(value: Option<&Json>) -> Scalar<'_> {
    match value {
        Some(Json::Number(number)) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => Scalar::Integer(integer),
            (None, Some(float)) => Scalar::Float(float),
            (None, None) => Scalar::Other,
        },
        Some(Json::Text(text)) => Scalar::Text(text.as_str()),
        Some(Json::Null)
        | Some(Json::Bool(_))
        | Some(Json::List(_))
        | Some(Json::Object(_))
        | None => Scalar::Other,
    }
}

#[test]
fn deployment_limits_loaded() {
    let source = "\
model_list:
- model_name: gpt-4
  litellm_params: {model: openai/gpt-4, default_api_key_rpm_limit: 10, default_api_key_tpm_limit: 100}
- model_name: gpt-4
  litellm_params: {model: openai/gpt-4, default_api_key_rpm_limit: \"20\"}
- model_name: gpt-3
  litellm_params: {model: openai/gpt-3, default_api_key_tpm_limit: 0}
- model_name: broken
  litellm_params: {model: openai/broken, default_api_key_rpm_limit: not-a-number, default_api_key_tpm_limit: 1.5}
- model_name: cleared
  litellm_params: {model: openai/cleared, default_api_key_rpm_limit: null}
- model_name: absent
  litellm_params: {model: openai/absent}
";
    let loaded = settings(source).unwrap();
    assert_eq!(loaded.deployments.len(), 6);
    let first = &loaded.deployments[0];
    assert_eq!(
        first,
        &DeploymentLimitDefaults {
            model_name: "gpt-4".to_owned(),
            default_api_key_rpm_limit: Some(Json::Number(serde_json::Number::from(10))),
            default_api_key_tpm_limit: Some(Json::Number(serde_json::Number::from(100))),
        }
    );
    assert_eq!(
        scalar(loaded.deployments[0].default_api_key_rpm_limit.as_ref()),
        Scalar::Integer(10)
    );
    assert_eq!(
        scalar(loaded.deployments[0].default_api_key_tpm_limit.as_ref()),
        Scalar::Integer(100)
    );
    assert_eq!(
        scalar(loaded.deployments[1].default_api_key_rpm_limit.as_ref()),
        Scalar::Text("20")
    );
    assert_eq!(
        scalar(loaded.deployments[1].default_api_key_tpm_limit.as_ref()),
        Scalar::Other
    );
    assert_eq!(
        scalar(loaded.deployments[2].default_api_key_rpm_limit.as_ref()),
        Scalar::Other
    );
    assert_eq!(
        scalar(loaded.deployments[2].default_api_key_tpm_limit.as_ref()),
        Scalar::Integer(0)
    );
    assert_eq!(
        scalar(loaded.deployments[3].default_api_key_rpm_limit.as_ref()),
        Scalar::Text("not-a-number")
    );
    assert_eq!(
        scalar(loaded.deployments[3].default_api_key_tpm_limit.as_ref()),
        Scalar::Float(1.5)
    );
    assert_eq!(loaded.deployments[4].default_api_key_rpm_limit, None);
    assert_eq!(loaded.deployments[5].default_api_key_rpm_limit, None);
    assert_eq!(loaded.deployments[5].default_api_key_tpm_limit, None);
}

#[test]
fn unrelated_deployment_default_keys_stay_unsupported() {
    let source = "model_list:\n- model_name: gpt-4\n  litellm_params: {model: openai/gpt-4, default_api_key_rpm_limitt: 10}\n";
    let failure = settings(source).unwrap_err();
    assert!(matches!(failure, ConfigError::Unsupported { .. }));
}

#[test]
fn no_model_list_loads_an_empty_deployment_list() {
    assert_eq!(
        settings("general_settings: {}").unwrap(),
        Settings::default()
    );
}
