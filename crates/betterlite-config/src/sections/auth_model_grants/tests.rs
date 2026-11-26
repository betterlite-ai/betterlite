#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn fallback_access_default_false() {
    for source in [
        "{}",
        "general_settings: {}",
        "general_settings: {master_key: sk}",
    ] {
        let settings = ConfigLoader {
            lookup: BTreeMap::<String, String>::new(),
        }
        .load(source)
        .expect("the config loads")
        .sections
        .auth_model_grants;
        assert!(!settings.enforce_fallback_model_access, "{source}");
    }
}

#[test]
fn fallback_access_enabled() {
    let settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings: {enforce_fallback_model_access: true}")
    .expect("the config loads")
    .sections
    .auth_model_grants;
    assert!(settings.enforce_fallback_model_access);
    // an explicit false keeps the pinned default and is not an error
    let disabled = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings: {enforce_fallback_model_access: false}")
    .expect("the config loads")
    .sections
    .auth_model_grants;
    assert!(!disabled.enforce_fallback_model_access);
}

#[test]
fn invalid_fallback_bool() {
    for source in [
        "general_settings: {enforce_fallback_model_access: []}",
        "general_settings: {enforce_fallback_model_access: \"true\"}",
    ] {
        let error = ConfigLoader {
            lookup: BTreeMap::<String, String>::new(),
        }
        .load(source)
        .expect_err("a value that is not a boolean is rejected");
        assert!(
            error.to_string().contains("enforce_fallback_model_access"),
            "{error}"
        );
    }
}
