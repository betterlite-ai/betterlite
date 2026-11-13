use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_content_safety::api_version::DEFAULT_API_VERSION;
use crate::azure_content_safety::base::AzureGuardrailBase;
use crate::azure_content_safety::config::AzureContentSafetyConfigModel;

fn config(json: &str) -> AzureContentSafetyConfigModel {
    serde_json::from_str(json).unwrap()
}

#[test]
fn the_config_model_defaults_the_api_version() {
    let model = config("{}");
    assert!(model.api_key.is_none());
    assert!(model.api_base.is_none());
    assert_eq!(model.api_version.as_deref(), Some(DEFAULT_API_VERSION));
}

#[test]
fn the_config_model_keeps_an_explicit_null_api_version() {
    let model = config(r#"{"api_version":null}"#);
    assert_eq!(model.api_version, None);
}

#[test]
fn the_config_model_keeps_an_explicitly_empty_api_version() {
    let model = config(r#"{"api_version":""}"#);
    assert_eq!(model.api_version.as_deref(), Some(""));
}

#[test]
fn the_config_model_reads_all_three_fields() {
    let model = config(
        r#"{"api_key":"azure-secret","api_base":"https://resource.cognitiveservices.azure.com","api_version":"2023-10-01"}"#,
    );
    assert_eq!(model.api_key.unwrap().expose_secret(), "azure-secret");
    assert_eq!(
        model.api_base.as_deref(),
        Some("https://resource.cognitiveservices.azure.com")
    );
    assert_eq!(model.api_version.as_deref(), Some("2023-10-01"));
}

#[test]
fn the_config_model_default_matches_its_missing_fields() {
    let model = AzureContentSafetyConfigModel::default();
    assert_eq!(model.api_version, config("{}").api_version);
}

#[test]
fn the_base_keeps_the_fields_it_was_built_with() {
    let guardrail_base = AzureGuardrailBase {
        async_handler: base_builder().build().unwrap(),
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: "https://resource.cognitiveservices.azure.com".to_owned(),
        api_version: Some("2023-10-01".to_owned()),
        timeout: None,
    };
    assert_eq!(guardrail_base.api_key.expose_secret(), "azure-key");
    assert_eq!(
        guardrail_base.api_base,
        "https://resource.cognitiveservices.azure.com"
    );
    assert_eq!(guardrail_base.api_version.as_deref(), Some("2023-10-01"));
    assert_eq!(guardrail_base.timeout, None);
}

#[test]
fn the_base_keeps_an_absent_version_for_the_transport_to_resolve() {
    let guardrail_base = AzureGuardrailBase {
        async_handler: base_builder().build().unwrap(),
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: "https://resource.cognitiveservices.azure.com".to_owned(),
        api_version: None,
        timeout: None,
    };
    assert_eq!(guardrail_base.api_version, None);
}
