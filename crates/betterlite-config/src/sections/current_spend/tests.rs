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

#[test]
fn strict_default_false() {
    let config = load("model_list: []").unwrap();
    assert!(!config.sections.current_spend.fail_closed_budget_enforcement);
}

#[test]
fn strict_true_consumer() {
    let config = load("general_settings: {fail_closed_budget_enforcement: true}").unwrap();
    assert!(config.sections.current_spend.fail_closed_budget_enforcement);
}

#[rstest]
#[case::explicit_false("general_settings: {fail_closed_budget_enforcement: false}")]
#[case::null("general_settings: {fail_closed_budget_enforcement: null}")]
#[case::absent_key("general_settings: {}")]
#[case::absent_section("{}")]
fn false_and_absent_preserve_fallback(#[case] source: &str) {
    let config = load(source).unwrap();
    assert!(!config.sections.current_spend.fail_closed_budget_enforcement);
}

#[rstest]
#[case::text("general_settings: {fail_closed_budget_enforcement: \"true\"}")]
#[case::number("general_settings: {fail_closed_budget_enforcement: 1}")]
#[case::list("general_settings: {fail_closed_budget_enforcement: [true]}")]
fn invalid_bool_rejected(#[case] source: &str) {
    let error = load(source).unwrap_err();
    assert!(
        matches!(
            &error,
            ConfigError::InvalidScalar { path, value: false }
                if path == "general_settings.fail_closed_budget_enforcement"
        ),
        "{error}"
    );
}
