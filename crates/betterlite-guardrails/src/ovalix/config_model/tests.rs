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
fn the_config_model_accepts_optional_params_as_null_or_an_object() {
    let absent: OvalixGuardrailConfigModel = serde_json::from_str("{}").expect("model");
    assert!(absent.optional_params.is_none());
    let null: OvalixGuardrailConfigModel =
        serde_json::from_str(r#"{"optional_params": null}"#).expect("model");
    assert!(null.optional_params.is_none());
    let provided: OvalixGuardrailConfigModel =
        serde_json::from_str(r#"{"optional_params": {"on_flagged_action": "block"}}"#)
            .expect("model");
    assert!(provided.optional_params.is_some());
}

#[test]
fn the_config_model_describes_every_field_the_way_litellm_does() {
    assert_eq!(
        OvalixGuardrailConfigModel::TRACKER_API_BASE_DESCRIPTION,
        "Base URL for the Ovalix Tracker service."
    );
    assert_eq!(
        OvalixGuardrailConfigModel::TRACKER_API_KEY_DESCRIPTION,
        "API key for the Ovalix Tracker service."
    );
    assert_eq!(
        OvalixGuardrailConfigModel::APPLICATION_ID_DESCRIPTION,
        "Application ID for the Ovalix Tracker service."
    );
    assert_eq!(
        OvalixGuardrailConfigModel::PRE_CHECKPOINT_ID_DESCRIPTION,
        "Pre-checkpoint ID for the Ovalix Tracker service."
    );
    assert_eq!(
        OvalixGuardrailConfigModel::POST_CHECKPOINT_ID_DESCRIPTION,
        "Post-checkpoint ID for the Ovalix Tracker service."
    );
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
