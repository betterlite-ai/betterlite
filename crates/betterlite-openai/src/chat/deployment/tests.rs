#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::ModelDeployment;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{ChatDeployment, DeploymentError, OpenAiDefaults, Provider};

fn deployment(entry: &str) -> ModelDeployment {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let config = loader.load(&format!("model_list:\n  - {entry}\n")).unwrap();
    config.model_list.into_iter().next().unwrap()
}

fn defaults() -> OpenAiDefaults {
    OpenAiDefaults {
        request_timeout: Some(45.5),
        api_bases: vec![String::new(), "http://settings/v2/".to_owned()],
        api_keys: vec![SecretString::from("settings-key")],
        organization: Some("org-1".to_owned()),
    }
}

#[test]
fn resolves_the_deployment_as_configured() {
    let entry = "{model_name: g, litellm_params: {model: openai/m, api_base: 'http://h/v1?x=1', api_key: k, timeout: 2.5, rpm: 10}}";
    let resolved = ChatDeployment::resolve(&deployment(entry), &defaults()).unwrap();
    assert_eq!(resolved.model_group, "g");
    assert_eq!(resolved.litellm_model, "openai/m");
    assert_eq!(
        resolved.provider,
        Provider::OpenAi {
            model: "m".to_owned()
        }
    );
    assert_eq!(resolved.api_base, "http://h/v1");
    assert_eq!(
        resolved.endpoint.as_str(),
        "http://h/v1/chat/completions?x=1"
    );
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "k");
    assert_eq!(resolved.organization.as_deref(), Some("org-1"));
    assert_eq!(resolved.timeout, Duration::from_millis(2500));
    assert_eq!(resolved.timeout_seconds.to_string(), "2.5");
}

#[test]
fn falls_back_to_settings_then_to_litellm_defaults() {
    let entry = "{model_name: g, litellm_params: {model: openai/m}}";
    let resolved = ChatDeployment::resolve(&deployment(entry), &defaults()).unwrap();
    assert_eq!(
        resolved.endpoint.as_str(),
        "http://settings/v2/chat/completions"
    );
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "settings-key");
    assert_eq!(resolved.timeout_seconds.to_string(), "45.5");
    let bare = ChatDeployment::resolve(&deployment(entry), &OpenAiDefaults::default()).unwrap();
    assert_eq!(
        bare.endpoint.as_str(),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(bare.api_base, "https://api.openai.com/v1");
    assert!(bare.api_key.is_none());
    assert_eq!(bare.timeout, Duration::from_mins(100));
}

#[test]
fn keeps_other_providers_for_a_clear_refusal() {
    let entry = "{model_name: g, litellm_params: {model: azure/m}}";
    let resolved = ChatDeployment::resolve(&deployment(entry), &defaults()).unwrap();
    assert_eq!(
        resolved.provider,
        Provider::Unsupported {
            model: "azure/m".to_owned()
        }
    );
}

#[rstest]
#[case::bad_api_base("{model_name: g, litellm_params: {model: openai/m, api_base: 'not a url'}}")]
#[case::negative_timeout("{model_name: g, litellm_params: {model: openai/m, timeout: -1}}")]
fn refuses_deployments_that_cannot_be_called(#[case] entry: &str) {
    let error = ChatDeployment::resolve(&deployment(entry), &defaults()).unwrap_err();
    assert!(matches!(
        error,
        DeploymentError::ApiBase { .. } | DeploymentError::Timeout { .. }
    ));
    assert!(error.to_string().contains("model_list entry g"));
}

#[rstest]
#[case::litellm_oracle_hash(
    "{model_name: oracle-model, litellm_params: {model: openai/oracle-model, api_base: 'http://127.0.0.1:40985', api_key: oracle-key}}",
    "ad74a715c2e662440c2597e7713f81cb31cbcdb7d139bfa2ca9bd42dd0ca28c7"
)]
#[case::numbers_join_the_hash(
    "{model_name: g, litellm_params: {model: openai/m, api_base: 'http://h/v1', timeout: 2.5, rpm: 10}}",
    "efa3c14c049d9858bed00cb57bca6596797e7ae2651990121c79a959f20309c0"
)]
#[case::configured_id_wins(
    "{model_name: g, litellm_params: {model: openai/m}, model_info: {id: fixed-id}}",
    "fixed-id"
)]
fn derives_the_model_id_like_litellm(#[case] entry: &str, #[case] id: &str) {
    let resolved = ChatDeployment::resolve(&deployment(entry), &OpenAiDefaults::default()).unwrap();
    assert_eq!(resolved.model_id, id);
}
