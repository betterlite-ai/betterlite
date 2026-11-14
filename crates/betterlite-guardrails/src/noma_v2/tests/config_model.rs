use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_guardrails::noma_scan_policy::config::NomaV2GuardrailConfigModel;

#[test]
fn the_config_model_ui_friendly_name_is_noma_security_v2() {
    assert_eq!(
        NomaV2GuardrailConfigModel::ui_friendly_name(),
        "Noma Security v2"
    );
    assert_eq!(NomaV2GuardrailConfigModel::default().monitor_mode, None);
}

#[test]
fn the_config_model_reads_every_noma_v2_key() {
    let model: NomaV2GuardrailConfigModel = serde_json::from_str(
        r#"{
            "api_key": "key-1",
            "api_base": "https://noma.example",
            "application_id": "app-1",
            "gateway_name": "gw-1",
            "monitor_mode": true,
            "block_failures": false
        }"#,
    )
    .unwrap();
    assert_eq!(model.api_base.as_deref(), Some("https://noma.example"));
    assert_eq!(model.application_id.as_deref(), Some("app-1"));
    assert_eq!(model.gateway_name.as_deref(), Some("gw-1"));
    assert_eq!(model.monitor_mode, Some(true));
    assert_eq!(model.block_failures, Some(false));
}
