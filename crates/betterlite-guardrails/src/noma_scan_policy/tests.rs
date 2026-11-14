#![cfg(test)]

mod anonymization;
mod records;
mod support;
mod transport;
mod verdict;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;

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
    let absent = NomaGuardrailConfigModel {
        use_v2: Some(false),
        api_key: None,
        api_base: None,
        application_id: None,
    };
    assert_eq!(default_use_v2(), Some(false));
    assert_eq!(
        serde_json::from_str::<NomaGuardrailConfigModel>("{}").unwrap(),
        absent
    );
    assert_eq!(
        serde_json::from_str::<NomaGuardrailConfigModel>(r#"{"use_v2": null}"#).unwrap(),
        NomaGuardrailConfigModel {
            use_v2: None,
            ..absent
        }
    );
}

#[test]
fn legacy_config_reads_every_field() {
    let text = r#"{"use_v2": true, "api_key": "key", "api_base": "https://base/", "application_id": "app"}"#;
    assert_eq!(
        serde_json::from_str::<NomaGuardrailConfigModel>(text).unwrap(),
        NomaGuardrailConfigModel {
            use_v2: Some(true),
            api_key: Some(String::from("key")),
            api_base: Some(String::from("https://base/")),
            application_id: Some(String::from("app")),
        }
    );
}

#[test]
fn v2_config_reads_every_field_and_defaults_to_none() {
    let text = r#"{"api_key": "key", "api_base": "https://base/", "application_id": "app", "gateway_name": "gw", "monitor_mode": true, "block_failures": false}"#;
    assert_eq!(
        serde_json::from_str::<NomaV2GuardrailConfigModel>(text).unwrap(),
        NomaV2GuardrailConfigModel {
            api_key: Some(String::from("key")),
            api_base: Some(String::from("https://base/")),
            application_id: Some(String::from("app")),
            gateway_name: Some(String::from("gw")),
            monitor_mode: Some(true),
            block_failures: Some(false),
        }
    );
    assert_eq!(
        serde_json::from_str::<NomaV2GuardrailConfigModel>("{}").unwrap(),
        NomaV2GuardrailConfigModel::default()
    );
}
