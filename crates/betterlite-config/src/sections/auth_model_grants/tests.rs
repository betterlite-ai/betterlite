#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
}

#[rstest]
#[case::empty("{}")]
#[case::empty_general_settings("general_settings: {}")]
#[case::unrelated_setting("general_settings: {master_key: sk}")]
fn fallback_access_default_false(#[case] source: &str) {
    let settings = load(source)
        .expect("the config loads")
        .sections
        .auth_model_grants;
    assert!(!settings.enforce_fallback_model_access, "{source}");
}

#[test]
fn fallback_access_enabled() {
    let settings = load("general_settings: {enforce_fallback_model_access: true}")
        .expect("the config loads")
        .sections
        .auth_model_grants;
    assert!(settings.enforce_fallback_model_access);
    // an explicit false keeps the pinned default and is not an error
    let disabled = load("general_settings: {enforce_fallback_model_access: false}")
        .expect("the config loads")
        .sections
        .auth_model_grants;
    assert!(!disabled.enforce_fallback_model_access);
}

#[rstest]
#[case::wrong_type_list("general_settings: {enforce_fallback_model_access: []}")]
#[case::quoted_string("general_settings: {enforce_fallback_model_access: \"true\"}")]
fn invalid_fallback_bool(#[case] source: &str) {
    let error = load(source).expect_err("a value that is not a boolean is rejected");
    assert!(
        error.to_string().contains("enforce_fallback_model_access"),
        "{error}"
    );
}
