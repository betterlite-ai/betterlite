use http::header::{HeaderMap, HeaderValue};

use super::support::{DEFAULT_BASE, anthropic_config, beta_headers, header_values};
use crate::skills::config::BaseSkillsApiConfig as _;

fn validated(caller: HeaderMap) -> HeaderMap {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let mut headers = caller;
    config
        .validate_environment(&mut headers, None, None)
        .unwrap();
    headers
}

#[test]
fn skills_beta_merge_sets_the_beta_when_absent() {
    let headers = validated(HeaderMap::new());
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
    assert_eq!(header_values(&headers, "anthropic-version"), ["2023-06-01"]);
    assert_eq!(
        header_values(&headers, "content-type"),
        ["application/json"]
    );
}

#[test]
fn skills_beta_merge_appends_to_a_caller_string_missing_the_version() {
    let headers = validated(beta_headers(&["caller-beta"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["caller-beta", "skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_keeps_a_caller_string_containing_the_version() {
    let headers = validated(beta_headers(&["skills-2025-10-02"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_tests_a_single_value_by_containment() {
    let headers = validated(beta_headers(&["skills-2025-10-02-beta"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["skills-2025-10-02-beta"]
    );
}

#[test]
fn skills_beta_merge_appends_to_a_caller_list_missing_the_version() {
    let headers = validated(beta_headers(&["beta-a", "beta-b"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["beta-a", "beta-b", "skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_keeps_a_caller_list_containing_the_version() {
    let headers = validated(beta_headers(&["beta-a", "skills-2025-10-02"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["beta-a", "skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_tests_a_list_by_exact_membership() {
    let headers = validated(beta_headers(&["skills-2025-10-02-beta", "beta-b"]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["skills-2025-10-02-beta", "beta-b", "skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_appends_to_an_empty_caller_string() {
    let headers = validated(beta_headers(&[""]));
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["", "skills-2025-10-02"]
    );
}

#[test]
fn skills_beta_merge_preserves_the_caller_headers() {
    let mut caller = HeaderMap::new();
    caller.insert("anthropic-beta", HeaderValue::from_static("caller-beta"));
    caller.insert("anthropic-version", HeaderValue::from_static("old-version"));
    caller.insert("x-api-key", HeaderValue::from_static("old-key"));
    caller.insert("x-keep", HeaderValue::from_static("keep"));
    let headers = validated(caller);
    assert_eq!(header_values(&headers, "x-keep"), ["keep"]);
    assert_eq!(header_values(&headers, "anthropic-version"), ["2023-06-01"]);
    assert_eq!(header_values(&headers, "x-api-key"), ["sk-ant-key"]);
    assert_eq!(
        header_values(&headers, "anthropic-beta"),
        ["caller-beta", "skills-2025-10-02"]
    );
}
