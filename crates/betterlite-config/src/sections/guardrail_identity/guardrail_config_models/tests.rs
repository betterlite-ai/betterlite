#![cfg(test)]

use std::collections::BTreeMap;

use super::super::config_models::{
    DeepKeepGuardrailConfigModel, GuardrailConfigModelIdentity as _, LassoGuardrailConfigModel,
    PresidioConfigModel,
};
use crate::loader::{ConfigError, ConfigLoader};

#[test]
fn identity_section_preserves_environment_errors() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let error = loader
        .load("general_settings:\n  master_key: os.environ/MISSING_MASTER_KEY\n")
        .unwrap_err();
    assert!(
        matches!(error, ConfigError::MissingEnvironment { variable, path }
        if variable == "MISSING_MASTER_KEY" && path == "general_settings.master_key")
    );
}

#[test]
fn defaults_are_preserved() {
    let presidio: PresidioConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(presidio.presidio_language.as_deref(), Some("en"));
    let lasso: LassoGuardrailConfigModel =
        serde_json::from_str("{\"lasso_user_id\":null,\"lasso_conversation_id\":null}").unwrap();
    assert_eq!(lasso.mask, Some(false));
    let deepkeep: DeepKeepGuardrailConfigModel =
        serde_json::from_str("{\"deepkeep_firewall_id\":null}").unwrap();
    assert_eq!(deepkeep.deepkeep_firewall_id, None);
}

#[test]
fn concrete_models_provide_their_identity() {
    assert_eq!(
        PresidioConfigModel::ui_friendly_name(),
        "Presidio Guardrail"
    );
    assert_eq!(
        LassoGuardrailConfigModel::ui_friendly_name(),
        "Lasso Guardrail"
    );
    assert_eq!(
        DeepKeepGuardrailConfigModel::ui_friendly_name(),
        "DeepKeep AI Firewall"
    );
}

#[test]
fn arbitrary_json_values_are_preserved() {
    let model: PresidioConfigModel =
        serde_json::from_str(r#"{"mock_redacted_text":{"text":"masked","items":[]}}"#).unwrap();
    assert!(matches!(
        model
            .mock_redacted_text
            .as_ref()
            .and_then(|values| values.get("items")),
        Some(super::super::item::GuardrailValue::Array(items)) if items.is_empty()
    ));
}
