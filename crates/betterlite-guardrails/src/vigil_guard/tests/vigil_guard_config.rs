use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_guardrails::vigil_guard::vigil_guard_config::VigilGuardGuardrailConfigModel;

#[test]
fn deserialize_api_fields() {
    let model: VigilGuardGuardrailConfigModel = serde_json::from_str(
        r#"{"api_base":"https://vigil.example","api_key":"sk-vigil","unreachable_fallback":"fail_open"}"#,
    )
    .unwrap();
    assert_eq!(model.api_base.as_deref(), Some("https://vigil.example"));
    assert_eq!(model.api_key.as_deref(), Some("sk-vigil"));
}

#[test]
fn round_trip_api_key() {
    let model = VigilGuardGuardrailConfigModel {
        api_base: None,
        api_key: Some("sk-vigil".to_owned()),
    };
    let encoded = serde_json::to_string(&model).unwrap();
    let decoded: VigilGuardGuardrailConfigModel = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.api_key.as_deref(), Some("sk-vigil"));
    assert_eq!(decoded.api_base, None);
    assert_eq!(
        VigilGuardGuardrailConfigModel::ui_friendly_name(),
        "Vigil Guard"
    );
}

#[test]
fn round_trip_api_base() {
    let model = VigilGuardGuardrailConfigModel {
        api_base: Some("https://vigil.example".to_owned()),
        api_key: None,
    };
    let encoded = serde_json::to_string(&model).unwrap();
    let decoded: VigilGuardGuardrailConfigModel = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.api_base.as_deref(), Some("https://vigil.example"));
    assert_eq!(decoded.api_key, None);
}
