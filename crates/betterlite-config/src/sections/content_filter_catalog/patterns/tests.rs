#![cfg(test)]

use std::error::Error as _;

use rstest::rstest;

use super::{
    PatternCatalogError, get_all_pattern_names, get_compiled_pattern, get_pattern_metadata,
    pattern_categories, pattern_extra_config,
};

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
fn get_compiled_pattern_matches_python_unicode_casefold_and_word_boundaries() {
    let ssn = get_compiled_pattern("us_ssn").unwrap();
    assert!(ssn.is_match("123-45-6789\u{0301}").unwrap());
    assert!(ssn.is_match("123-45-6789\u{0345}").unwrap());
    let aws_key = get_compiled_pattern("aws_access_key").unwrap();
    assert!(aws_key.is_match("AK\u{130}A1234567890ABCDEF").unwrap());
}

#[rstest]
#[case::file_separator('\u{1c}')]
#[case::group_separator('\u{1d}')]
#[case::record_separator('\u{1e}')]
#[case::unit_separator('\u{1f}')]
fn get_compiled_visa_pattern_matches_python_control_whitespace(#[case] separator: char) {
    let visa = get_compiled_pattern("visa").unwrap();
    let number = format!("4111{separator}1111{separator}1111{separator}1111");
    assert!(visa.is_match(&number).unwrap(), "separator {separator:?}");
}

#[rstest]
#[case::outside_file_separator(r"\s", '\u{1c}')]
#[case::outside_group_separator(r"\s", '\u{1d}')]
#[case::outside_record_separator(r"\s", '\u{1e}')]
#[case::outside_unit_separator(r"\s", '\u{1f}')]
#[case::inside_file_separator(r"[\s]", '\u{1c}')]
#[case::inside_group_separator(r"[\s]", '\u{1d}')]
#[case::inside_record_separator(r"[\s]", '\u{1e}')]
#[case::inside_unit_separator(r"[\s]", '\u{1f}')]
fn compatible_whitespace_escape_matches_python_controls_inside_and_outside_classes(
    #[case] source: &str,
    #[case] whitespace: char,
) {
    let pattern = super::compatible_regex_pattern(source);
    let regex = fancy_regex::Regex::new(&format!("(?i){pattern}")).unwrap();
    assert!(regex.is_match(&whitespace.to_string()).unwrap());
}

#[test]
fn pattern_errors_preserve_typed_sources() {
    let parse_source = serde_json::from_str::<super::PatternsFile>("{").unwrap_err();
    let parse_error = PatternCatalogError::Parse {
        source: parse_source,
    };
    assert!(parse_error.source().is_some());
    let compile_source = fancy_regex::Regex::new("(").unwrap_err();
    let compile_error = PatternCatalogError::Compile {
        name: "broken".to_owned(),
        source: compile_source,
    };
    assert!(compile_error.source().is_some());
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
