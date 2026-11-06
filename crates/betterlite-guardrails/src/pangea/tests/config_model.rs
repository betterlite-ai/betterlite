#![cfg(test)]

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;

use crate::pangea::PangeaGuardrailConfigModel;

#[test]
fn config_model_deserializes_api_key_api_base_and_recipes() {
    let model: PangeaGuardrailConfigModel = serde_json::from_str(
        r#"{"api_key":"key","api_base":"https://pangea.test","pangea_input_recipe":"in","pangea_output_recipe":"out"}"#,
    )
    .unwrap();
    assert_eq!(model.api_key.as_deref(), Some("key"));
    assert_eq!(model.api_base.as_deref(), Some("https://pangea.test"));
    assert_eq!(model.pangea_input_recipe.as_deref(), Some("in"));
    assert_eq!(model.pangea_output_recipe.as_deref(), Some("out"));
}

#[test]
fn config_model_defaults_every_field_to_none() {
    let model: PangeaGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.api_key, None);
    assert_eq!(model.api_base, None);
    assert_eq!(model.pangea_input_recipe, None);
    assert_eq!(model.pangea_output_recipe, None);
}

#[test]
fn config_model_rejects_an_unknown_field() {
    let model =
        serde_json::from_str::<PangeaGuardrailConfigModel>(r#"{"api_key":"key","unknown":true}"#);
    assert!(model.is_err());
}

#[test]
fn config_model_ui_friendly_name_is_pangea_guardrail() {
    assert_eq!(
        PangeaGuardrailConfigModel::ui_friendly_name(),
        "Pangea Guardrail"
    );
}
