#![cfg(test)]

use super::super::config_models::{
    DeepKeepGuardrailConfigModel, LassoGuardrailConfigModel, PresidioConfigModel,
};

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
