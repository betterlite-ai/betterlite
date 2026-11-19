#![cfg(test)]

use crate::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::absent("{}", None)]
#[case::configured(
    "general_settings: {moderation_model: omni-moderation-latest}",
    Some("omni-moderation-latest")
)]
#[case::null("general_settings: {moderation_model: null}", None)]
fn moderations_moderation_tests_behavior(#[case] source: &str, #[case] expected: Option<&str>) {
    let settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .moderation_model;
    assert_eq!(settings.moderation_model.as_deref(), expected);
}

#[test]
fn the_moderation_model_rejects_wrong_types() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings: {moderation_model: [omni-moderation-latest]}")
    .unwrap_err();
    assert!(error.to_string().contains("moderation_model"));
}
