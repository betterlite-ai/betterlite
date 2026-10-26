#![cfg(test)]

use std::collections::BTreeMap;

use super::super::config_models::{
    DeepKeepGuardrailConfigModel, LassoGuardrailConfigModel, PresidioConfigModel,
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
