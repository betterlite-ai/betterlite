#![cfg(test)]

use rstest::rstest;

use super::support::FixedIds;
use crate::azure_storage_logging::filename::{adls_safe_file_name, encode_path_segment};

const IDS: FixedIds = FixedIds;

#[rstest]
#[case::a_plain_id("abc", "abc.json")]
#[case::a_responses_id("resp_YWJj", "resp_YWJj.json")]
#[case::responses_slash_and_padding("resp_YWJj/ZA==", "resp_YWJj_ZA.json")]
#[case::a_slash_keeps_the_name("a/b", "a_b.json")]
#[case::an_empty_segment("a//b", "a__b.json")]
#[case::a_dot_segment("./a", "._a.json")]
#[case::a_dot_dot_segment("../a", ".._a.json")]
#[case::percent_encoded_slash("a%2Fb", "a%2Fb.json")]
#[case::percent_encoded_dot("a/%2E%2E", "a_%2E%2E.json")]
fn the_safe_file_name_sanitizes_the_payload_id(#[case] payload_id: &str, #[case] expected: &str) {
    assert_eq!(adls_safe_file_name(Some(payload_id), &IDS), expected);
}

#[test]
fn a_missing_id_gets_a_generated_name() {
    assert_eq!(adls_safe_file_name(None, &IDS), "generated-id.json");
}

#[test]
fn an_empty_id_is_kept_as_given() {
    assert_eq!(adls_safe_file_name(Some(""), &IDS), ".json");
}

#[test]
fn a_path_segment_is_percent_encoded_for_the_url() {
    assert_eq!(encode_path_segment("a b"), "a%20b");
    assert_eq!(
        encode_path_segment("resp_YWJj_ZA.json"),
        "resp_YWJj_ZA.json"
    );
}
