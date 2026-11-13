#![cfg(test)]

use std::time::Duration;

use secrecy::SecretString;

use super::{DEFAULT_REQUEST_TIMEOUT, RawXecGuardSettings, XecGuardSettings};

fn raw() -> RawXecGuardSettings {
    RawXecGuardSettings {
        api_key: SecretString::from("key"),
        api_base: "https://xecguard.test".to_owned(),
        model: "xecguard_v2".to_owned(),
        policy_names: None,
        block_on_error: false,
        grounding_strictness: "BALANCED".to_owned(),
        timeout: None,
    }
}

#[test]
fn settings_resolve_the_default_policies_and_timeout_once() {
    let settings = XecGuardSettings::from(raw());
    assert_eq!(settings.timeout, DEFAULT_REQUEST_TIMEOUT);
    assert_eq!(
        settings.policy_names,
        [
            "Default_Policy_SystemPromptEnforcement",
            "Default_Policy_HarmfulContentProtection",
            "Default_Policy_GeneralPromptAttackProtection",
        ]
    );
}

#[test]
fn settings_keep_explicit_policies_and_timeout_as_given() {
    let mut input = raw();
    input.policy_names = Some(Vec::new());
    input.timeout = Some(Duration::from_secs(2));
    let settings = XecGuardSettings::from(input);
    assert!(settings.policy_names.is_empty());
    assert_eq!(settings.timeout, Duration::from_secs(2));
}
