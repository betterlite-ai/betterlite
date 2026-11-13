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
fn config_models_carry_litellm_fields_and_names() {
    assert_eq!(
        NomaGuardrailConfigModel::ui_friendly_name(),
        "Noma Security"
    );
    assert_eq!(
        NomaV2GuardrailConfigModel::ui_friendly_name(),
        "Noma Security v2"
    );
    assert_eq!(default_use_v2(), Some(false));
    let legacy: NomaGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.use_v2, Some(false));
    assert_eq!(legacy.api_key, None);
    assert_eq!(legacy.api_base, None);
    assert_eq!(legacy.application_id, None);
    let explicit: NomaGuardrailConfigModel = serde_json::from_str(
        r#"{"use_v2": true, "api_key": "key", "api_base": "https://base/", "application_id": "app"}"#,
    )
    .unwrap();
    assert_eq!(explicit.use_v2, Some(true));
    assert_eq!(explicit.api_key.as_deref(), Some("key"));
    assert_eq!(explicit.api_base.as_deref(), Some("https://base/"));
    assert_eq!(explicit.application_id.as_deref(), Some("app"));
    let null: NomaGuardrailConfigModel = serde_json::from_str(r#"{"use_v2": null}"#).unwrap();
    assert_eq!(null.use_v2, None);
    let v2: NomaV2GuardrailConfigModel = serde_json::from_str(
        r#"{"api_key": "key", "api_base": "https://base/", "application_id": "app", "gateway_name": "gw", "monitor_mode": true, "block_failures": false}"#,
    )
    .unwrap();
    assert_eq!(v2.api_key.as_deref(), Some("key"));
    assert_eq!(v2.api_base.as_deref(), Some("https://base/"));
    assert_eq!(v2.application_id.as_deref(), Some("app"));
    assert_eq!(v2.gateway_name.as_deref(), Some("gw"));
    assert_eq!(v2.monitor_mode, Some(true));
    assert_eq!(v2.block_failures, Some(false));
    let v2_default: NomaV2GuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert!(v2_default == NomaV2GuardrailConfigModel::default());
}
