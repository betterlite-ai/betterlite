#![cfg(test)]

mod anonymization;
mod records;
mod support;
mod transport;
mod verdict;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use secrecy::ExposeSecret;

use crate::noma_scan_policy::config::{
    NomaGuardrailConfigModel, NomaV2GuardrailConfigModel, default_use_v2,
};

#[test]
fn config_models_carry_litellm_names() {
    assert_eq!(
        NomaGuardrailConfigModel::ui_friendly_name(),
        "Noma Security"
    );
    assert_eq!(
        NomaV2GuardrailConfigModel::ui_friendly_name(),
        "Noma Security v2"
    );
}

#[test]
fn legacy_config_defaults_use_v2_to_false_and_keeps_an_explicit_null() {
    let absent = serde_json::from_str::<NomaGuardrailConfigModel>("{}").unwrap();
    let null = serde_json::from_str::<NomaGuardrailConfigModel>(r#"{"use_v2": null}"#).unwrap();
    assert_eq!(default_use_v2(), Some(false));
    assert_eq!(absent.use_v2, Some(false));
    assert_eq!(null.use_v2, None);
    assert!(
        absent.api_key.is_none() && absent.api_base.is_none() && absent.application_id.is_none()
    );
}

#[test]
fn legacy_config_reads_every_field() {
    let text = r#"{"use_v2": true, "api_key": "key", "api_base": "https://base/", "application_id": "app"}"#;
    let model = serde_json::from_str::<NomaGuardrailConfigModel>(text).unwrap();
    assert_eq!(model.use_v2, Some(true));
    assert_eq!(
        model.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("key")
    );
    assert_eq!(model.api_base.as_deref(), Some("https://base/"));
    assert_eq!(model.application_id.as_deref(), Some("app"));
}

#[test]
fn v2_config_reads_every_field_and_defaults_to_none() {
    let text = r#"{"api_key": "key", "api_base": "https://base/", "application_id": "app", "gateway_name": "gw", "monitor_mode": true, "block_failures": false}"#;
    let model = serde_json::from_str::<NomaV2GuardrailConfigModel>(text).unwrap();
    assert_eq!(
        model.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("key")
    );
    assert_eq!(model.api_base.as_deref(), Some("https://base/"));
    assert_eq!(model.application_id.as_deref(), Some("app"));
    assert_eq!(model.gateway_name.as_deref(), Some("gw"));
    assert_eq!(model.monitor_mode, Some(true));
    assert_eq!(model.block_failures, Some(false));
}

#[test]
fn v2_config_without_keys_is_all_none() {
    let model = serde_json::from_str::<NomaV2GuardrailConfigModel>("{}").unwrap();
    let default = NomaV2GuardrailConfigModel::default();
    assert!(model.api_key.is_none() && default.api_key.is_none());
    assert!(model.api_base.is_none() && model.application_id.is_none());
    assert!(model.gateway_name.is_none() && default.gateway_name.is_none());
    assert!(model.monitor_mode.is_none() && model.block_failures.is_none());
}
