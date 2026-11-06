#![cfg(test)]

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;

use super::OvalixGuardrailConfigModel;

#[test]
fn the_config_model_reads_every_field() {
    let model: OvalixGuardrailConfigModel = serde_json::from_str(
        r#"{
            "tracker_api_base": "http://tracker.test",
            "tracker_api_key": "oracle-key",
            "application_id": "app-1",
            "pre_checkpoint_id": "pre-1",
            "post_checkpoint_id": "post-1"
        }"#,
    )
    .expect("model");
    assert_eq!(
        model.tracker_api_base.as_deref(),
        Some("http://tracker.test")
    );
    assert_eq!(model.tracker_api_key.as_deref(), Some("oracle-key"));
    assert_eq!(model.application_id.as_deref(), Some("app-1"));
    assert_eq!(model.pre_checkpoint_id.as_deref(), Some("pre-1"));
    assert_eq!(model.post_checkpoint_id.as_deref(), Some("post-1"));
}

#[test]
fn the_config_model_defaults_every_field_to_none() {
    let model: OvalixGuardrailConfigModel = serde_json::from_str("{}").expect("model");
    assert!(model.tracker_api_base.is_none());
    assert!(model.tracker_api_key.is_none());
    assert!(model.application_id.is_none());
    assert!(model.pre_checkpoint_id.is_none());
    assert!(model.post_checkpoint_id.is_none());
}

#[test]
fn the_config_model_rejects_an_unknown_field() {
    let result: Result<OvalixGuardrailConfigModel, _> =
        serde_json::from_str(r#"{"tracker_url": "http://tracker.test"}"#);
    assert!(result.is_err());
}

#[test]
fn the_ui_friendly_name_is_ovalix_guardrail() {
    assert_eq!(
        OvalixGuardrailConfigModel::ui_friendly_name(),
        "Ovalix Guardrail"
    );
}
