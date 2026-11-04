#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn logging_privacy_tests_behavior() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  turn_off_message_logging: true\n  redact_user_api_key_info: true\n")
    .expect("privacy settings parse");
    assert!(config.sections.logging_privacy.turn_off_message_logging);
    assert!(config.sections.logging_privacy.redact_user_api_key_info);
}
