use std::collections::BTreeMap;

use super::support::threshold_map;
use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::legacy_thresholds::{
    DEFAULT_THRESHOLD, SeverityThresholds, configure_thresholds,
};
use crate::azure_content_safety::legacy_wire::TextCategory;

#[test]
fn an_empty_map_defaults_every_category_to_four() {
    let thresholds = configure_thresholds(BTreeMap::new()).unwrap();
    assert_eq!(thresholds.for_category(TextCategory::Hate), 4);
    assert_eq!(thresholds.for_category(TextCategory::SelfHarm), 4);
    assert_eq!(thresholds.for_category(TextCategory::Sexual), 4);
    assert_eq!(thresholds.for_category(TextCategory::Violence), 4);
    assert_eq!(DEFAULT_THRESHOLD, 4);
}

#[test]
fn keeps_every_provided_threshold() {
    let thresholds = configure_thresholds(threshold_map(&[
        ("Hate", 1),
        ("SelfHarm", 2),
        ("Sexual", 3),
        ("Violence", 5),
    ]))
    .unwrap();
    assert_eq!(
        thresholds,
        SeverityThresholds {
            hate: 1,
            self_harm: 2,
            sexual: 3,
            violence: 5,
        }
    );
}

#[test]
fn fills_only_the_categories_the_map_omits() {
    let thresholds =
        configure_thresholds(threshold_map(&[("Sexual", 6), ("Violence", 7)])).unwrap();
    assert_eq!(
        thresholds.for_category(TextCategory::Hate),
        DEFAULT_THRESHOLD
    );
    assert_eq!(
        thresholds.for_category(TextCategory::SelfHarm),
        DEFAULT_THRESHOLD
    );
    assert_eq!(thresholds.for_category(TextCategory::Sexual), 6);
    assert_eq!(thresholds.for_category(TextCategory::Violence), 7);
}

#[test]
fn keeps_an_explicit_zero_threshold() {
    let thresholds = configure_thresholds(threshold_map(&[("Hate", 0)])).unwrap();
    assert_eq!(thresholds.for_category(TextCategory::Hate), 0);
}

#[test]
fn keeps_a_negative_threshold_as_given() {
    let thresholds = configure_thresholds(threshold_map(&[("Hate", -1)])).unwrap();
    assert_eq!(thresholds.for_category(TextCategory::Hate), -1);
}

#[test]
fn rejects_an_unknown_category_and_names_it() {
    let error = configure_thresholds(threshold_map(&[("Hate", 2), ("Profanity", 3)])).unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::UnknownCategory { category } if category == "Profanity"
    ));
}

#[test]
fn a_category_name_is_matched_exactly() {
    let error = configure_thresholds(threshold_map(&[("hate", 2)])).unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::UnknownCategory { category } if category == "hate"
    ));
}
