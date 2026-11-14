use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{guardrail, guardrail_from, params, threshold_map};
use crate::azure_content_safety::base::DEFAULT_TIMEOUT;
use crate::azure_content_safety::legacy_wire::TextCategory;
use crate::azure_text_moderation::config::{AzureTextModerationConfigModel, OptionalParams};
use crate::azure_text_moderation::wire::OutputType;
use crate::azure_text_moderation::{AzureTextModerationGuardrail, AzureTextModerationParams};

fn hook_with(api_version: Option<&str>, timeout: Option<Duration>) -> AzureTextModerationGuardrail {
    guardrail_from(AzureTextModerationParams {
        api_key: SecretString::from("azure-key".to_owned()),
        api_base: "https://example.test".to_owned(),
        api_version: api_version.map(str::to_owned),
        timeout,
        optional_params: OptionalParams::default(),
    })
}

#[test]
fn stores_the_severity_thresholds() {
    let hook = guardrail(
        "https://example.test",
        params(Some(3), Some(threshold_map(&[("Hate", 1)]))),
    );
    assert_eq!(hook.severity_threshold, Some(3));
    assert_eq!(
        hook.severity_threshold_by_category,
        Some(threshold_map(&[("Hate", 1)]))
    );
}

#[test]
fn keeps_an_explicit_zero_threshold() {
    let hook = guardrail("https://example.test", params(Some(0), None));
    assert_eq!(hook.severity_threshold, Some(0));
}

#[test]
fn tells_an_absent_category_map_from_an_empty_one() {
    let absent = guardrail("https://example.test", params(None, None));
    assert_eq!(absent.severity_threshold_by_category, None);
    let empty = guardrail("https://example.test", params(None, Some(BTreeMap::new())));
    assert_eq!(empty.severity_threshold_by_category, Some(BTreeMap::new()));
}

#[test]
fn fills_the_default_request_body() {
    let hook = guardrail("https://example.test", OptionalParams::default());
    let body = hook.optional_params_request_body;
    assert_eq!(
        body.categories,
        vec![
            TextCategory::Hate,
            TextCategory::Sexual,
            TextCategory::SelfHarm,
            TextCategory::Violence
        ]
    );
    assert_eq!(body.blocklist_names, None);
    assert!(!body.halt_on_blocklist_hit);
    assert_eq!(body.output_type, OutputType::FourSeverityLevels);
}

#[test]
fn keeps_the_configured_request_body() {
    let optional_params = OptionalParams {
        severity_threshold: None,
        severity_threshold_by_category: None,
        categories: Some(vec![TextCategory::Violence]),
        blocklist_names: Some(vec!["list-a".to_owned()]),
        halt_on_blocklist_hit: Some(true),
        output_type: Some(OutputType::EightSeverityLevels),
    };
    let hook = guardrail("https://example.test", optional_params);
    let body = hook.optional_params_request_body;
    assert_eq!(body.categories, vec![TextCategory::Violence]);
    assert_eq!(body.blocklist_names, Some(vec!["list-a".to_owned()]));
    assert!(body.halt_on_blocklist_hit);
    assert_eq!(body.output_type, OutputType::EightSeverityLevels);
}

#[test]
fn resolves_the_api_version() {
    assert_eq!(hook_with(None, None).base.api_version, "2024-09-01");
    assert_eq!(hook_with(Some("v1"), None).base.api_version, "2024-09-01");
    assert_eq!(
        hook_with(Some("2023-10-01"), None).base.api_version,
        "2023-10-01"
    );
}

#[test]
fn takes_the_default_timeout_unless_configured() {
    assert_eq!(hook_with(None, None).base.timeout, DEFAULT_TIMEOUT);
    assert_eq!(
        hook_with(None, Some(Duration::from_secs(5))).base.timeout,
        Duration::from_secs(5)
    );
}

#[test]
fn stores_the_api_key_and_base() {
    let hook = hook_with(None, None);
    assert_eq!(hook.base.api_key.expose_secret(), "azure-key");
    assert_eq!(hook.base.api_base, "https://example.test");
}

#[test]
fn config_model_reads_the_optional_params() {
    let model: AzureTextModerationConfigModel = serde_json::from_str(
        r#"{"api_key":"key","api_base":"https://example.test","api_version":"v1",
            "optional_params":{"severity_threshold":4,
            "severity_threshold_by_category":{"Hate":1,"SelfHarm":3},
            "categories":["Violence"],"blocklistNames":["list-a"],
            "haltOnBlocklistHit":true,"outputType":"EightSeverityLevels"}}"#,
    )
    .unwrap();
    assert_eq!(model.base.api_version, "2024-09-01");
    assert_eq!(model.optional_params.severity_threshold, Some(4));
    assert_eq!(
        model.optional_params.severity_threshold_by_category,
        Some(threshold_map(&[("Hate", 1), ("SelfHarm", 3)]))
    );
    assert_eq!(
        model.optional_params.categories,
        Some(vec![TextCategory::Violence])
    );
    assert_eq!(
        model.optional_params.blocklist_names,
        Some(vec!["list-a".to_owned()])
    );
    assert_eq!(model.optional_params.halt_on_blocklist_hit, Some(true));
    assert_eq!(
        model.optional_params.output_type,
        Some(OutputType::EightSeverityLevels)
    );
}

#[test]
fn config_model_names_itself_in_the_ui() {
    assert_eq!(
        AzureTextModerationConfigModel::ui_friendly_name(),
        "Azure Content Safety Text Moderation"
    );
}
