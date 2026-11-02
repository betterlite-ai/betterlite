#![cfg(test)]

use super::{
    get_all_pattern_names, get_compiled_pattern, get_pattern_metadata, pattern_categories,
    pattern_extra_config,
};
use crate::sections::content_filter_catalog::patterns::PatternCatalogError;

#[test]
fn get_compiled_pattern_compiles_a_lookaround_pattern_from_the_bundled_json() {
    let pattern = get_compiled_pattern("us_ssn_no_dash").unwrap();
    assert!(pattern.is_match("123456789").unwrap());
    assert!(!pattern.is_match("000456789").unwrap());
}

#[test]
fn get_compiled_pattern_matches_case_insensitively() {
    let pattern = get_compiled_pattern("aws_access_key").unwrap();
    assert!(pattern.is_match("akia1234567890abcdef").unwrap());
}

#[test]
fn get_compiled_pattern_names_available_patterns_for_an_unknown_name() {
    let error = get_compiled_pattern("not_a_real_pattern").unwrap_err();
    let PatternCatalogError::UnknownPattern { name, available } = error else {
        panic!("expected an UnknownPattern error");
    };
    assert_eq!(name, "not_a_real_pattern");
    assert!(available.contains("us_ssn"));
}

#[test]
fn get_all_pattern_names_returns_every_bundled_pattern_in_source_order() {
    let names = get_all_pattern_names().unwrap();
    assert_eq!(names.len(), 82);
    assert_eq!(names[0], "us_ssn");
}

#[test]
fn get_pattern_metadata_returns_every_field_in_source_order() {
    let metadata = get_pattern_metadata().unwrap();
    assert_eq!(metadata.len(), 82);
    assert_eq!(metadata[0].name, "us_ssn");
    assert_eq!(metadata[0].display_name, "SSN (Social Security Number)");
    assert_eq!(metadata[0].category, "PII Patterns");
}

#[test]
fn pattern_categories_groups_pattern_names_under_their_category() {
    let categories = pattern_categories().unwrap();
    let pii = &categories["PII Patterns"];
    assert!(pii.contains(&"us_ssn".to_owned()));
}

#[test]
fn pattern_extra_config_defaults_to_empty_for_a_pattern_without_extras() {
    let extra = pattern_extra_config().unwrap();
    let us_ssn = &extra["us_ssn"];
    assert_eq!(us_ssn.keyword_pattern, None);
    assert_eq!(us_ssn.allow_word_numbers, None);
}

#[test]
fn pattern_extra_config_captures_declared_extras_for_a_contextual_pattern() {
    let extra = pattern_extra_config().unwrap();
    let nl_bsn = &extra["nl_bsn_contextual"];
    assert!(nl_bsn.keyword_pattern.is_some());
    assert_eq!(nl_bsn.allow_word_numbers, Some(true));
}
