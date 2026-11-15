#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::Agent365GuardrailConfigModel;

type Scenario = fn();

fn model(json: &str) -> Agent365GuardrailConfigModel {
    serde_json::from_str(json).unwrap()
}

fn defaults() {
    let parsed = model("{}");
    assert!(parsed.optional_params.is_none());
    assert!(parsed.tenant_id.is_none());
    assert!(parsed.client_id.is_none());
    assert!(parsed.client_secret.is_none());
    assert_eq!(parsed.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(
        serde_json::to_value(parsed.unreachable_fallback).unwrap(),
        serde_json::json!("fail_closed")
    );
    assert_eq!(
        Agent365GuardrailConfigModel::ui_friendly_name(),
        "Microsoft Agent 365"
    );
}

fn explicit_values() {
    let parsed = model(
        r#"{"tenant_id":"tenant-explicit","client_id":"client-explicit","client_secret":"secret-explicit","unreachable_fallback":"fail_open","optional_params":{"temperature":0.5}}"#,
    );
    assert_eq!(parsed.tenant_id.as_deref(), Some("tenant-explicit"));
    assert_eq!(parsed.client_id.as_deref(), Some("client-explicit"));
    assert_eq!(
        parsed
            .client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("secret-explicit")
    );
    assert_eq!(parsed.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(
        serde_json::to_value(parsed.unreachable_fallback).unwrap(),
        serde_json::json!("fail_open")
    );
    assert_eq!(
        parsed.optional_params,
        Some(GuardrailValue::Object(BTreeMap::from([(
            "temperature".to_owned(),
            GuardrailValue::Decimal(0.5),
        )])))
    );
}

fn unknown_keys_are_ignored() {
    let parsed = model(r#"{"tenant_id":"tenant","unknown_key":true}"#);
    assert_eq!(parsed.tenant_id.as_deref(), Some("tenant"));
}

fn unknown_unreachable_fallback_is_rejected() {
    let parsed = serde_json::from_str::<Agent365GuardrailConfigModel>(
        r#"{"unreachable_fallback": "fail_sideways"}"#,
    );
    assert!(parsed.is_err());
}

#[rstest]
#[case::defaults(defaults)]
#[case::explicit_values(explicit_values)]
#[case::unknown_keys_are_ignored(unknown_keys_are_ignored)]
#[case::unknown_unreachable_fallback_is_rejected(unknown_unreachable_fallback_is_rejected)]
fn agent_365_token_behaviors(#[case] scenario: Scenario) {
    scenario();
}
