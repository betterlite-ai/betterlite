use rstest::rstest;

use crate::azure_content_safety::api_version::{
    DEFAULT_API_VERSION, LEGACY_JAVELIN_VERSION, resolve_api_version,
};

#[test]
fn defaults_match_the_litellm_constants() {
    assert_eq!(DEFAULT_API_VERSION, "2024-09-01");
    assert_eq!(LEGACY_JAVELIN_VERSION, "v1");
}

#[test]
fn resolves_an_absent_version_to_the_modern_default() {
    assert_eq!(resolve_api_version(None), DEFAULT_API_VERSION);
}

#[test]
fn resolves_the_legacy_javelin_version_to_the_modern_default() {
    assert_eq!(resolve_api_version(Some("v1")), DEFAULT_API_VERSION);
}

#[rstest]
#[case::modern_default("2024-09-01")]
#[case::older_release("2023-10-01")]
#[case::future_release("2025-03-01")]
fn passes_every_other_version_through(#[case] configured: &str) {
    assert_eq!(resolve_api_version(Some(configured)), configured);
}

#[test]
fn keeps_an_explicit_empty_version_as_given() {
    // docs/compatibility.md: an explicit "" is a value, not an absent one.
    assert_eq!(resolve_api_version(Some("")), "");
}
