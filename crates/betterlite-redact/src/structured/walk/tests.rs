#![cfg(test)]

use super::{Context, Kind, Walker};
use crate::structured::{Payload, SensitiveDataMasker};
use rstest::rstest;
use std::collections::BTreeSet;

#[rstest]
#[case::depth(
    Kind::Config,
    1,
    100,
    r#"{"plain":{"token":"secret"}}"#,
    r#"{"plain":"REDACTED"}"#
)]
#[case::array_depth(
    Kind::Payload,
    1,
    100,
    r#"{"plain":["secret"]}"#,
    r#"{"plain":"REDACTED"}"#
)]
#[case::root_depth(Kind::Config, 0, 100, "{}", "\"REDACTED\"")]
#[case::null_root(Kind::Config, 10, 100, "null", "null")]
#[case::array_root(Kind::Config, 10, 100, "[null,true]", "[\"None\",true]")]
#[case::redact_nonstring(
    Kind::Redact,
    10,
    100,
    r#"{"token":false,"auth":null,"key":{}}"#,
    r#"{"token":"REDACTED","auth":null,"key":"REDACTED"}"#
)]
#[case::preserve_scalars(
    Kind::Payload,
    10,
    100,
    r#"{"key":[null,17,false,"short"]}"#,
    r#"{"key":[null,17,false,"*****"]}"#
)]
#[case::mapping_resets_sensitivity(
    Kind::Payload,
    10,
    100,
    r#"{"key":{"ordinary":"visible"}}"#,
    r#"{"key":{"ordinary":"visible"}}"#
)]
#[case::nested_array(
    Kind::Payload,
    10,
    100,
    r#"{"key":[["short"]]}"#,
    r#"{"key":[["*****"]]}"#
)]
#[case::byte_boundary(Kind::Payload, 10, 4, "\"x\"", "\"x\"")]
#[case::byte_equal(Kind::Payload, 10, 3, "\"x\"", "\"x\"")]
#[case::byte_exhausted(Kind::Payload, 10, 2, "\"x\"", "\"REDACTED\"")]
#[case::number_bytes(Kind::Payload, 10, 5, "12345", "\"REDACTED\"")]
#[case::number_equal(Kind::Payload, 10, 11, "12345", "12345")]
#[case::number_output(Kind::Payload, 10, 10, "12345", "\"REDACTED\"")]
#[case::zero_work(Kind::Payload, 10, 0, "{}", "\"REDACTED\"")]
#[case::key_bytes(Kind::Payload, 10, 2, r#"{"abc":null}"#, "\"REDACTED\"")]
#[case::exact(Kind::Exact(&BTreeSet::from(["token".to_owned()])), 10, 100, r#"{"token":"short","other":"short"}"#, r#"{"token":"*****","other":"short"}"#)]
#[case::exact_nonstring(Kind::Exact(&BTreeSet::from(["token".to_owned()])), 10, 100, r#"{"token":true}"#, r#"{"token":true}"#)]
#[case::exact_shallow(Kind::Exact(&BTreeSet::from(["token".to_owned()])), 10, 100, r#"{"token":{"token":"secret"}}"#, r#"{"token":{"token":"secret"}}"#)]
fn structured_redaction_behavior_tests_behavior(
    #[case] kind: Kind<'_>,
    #[case] depth: usize,
    #[case] work: usize,
    #[case] source: &str,
    #[case] expected: &str,
) {
    let input: Payload = serde_json::from_str(source).unwrap();
    let masker = SensitiveDataMasker::default();
    let mut walker = Walker {
        masker: &masker,
        kind,
        excluded_keys: &[],
        max_depth: depth,
        remaining: work,
    };
    let result = walker
        .walk(&input, 0, Context::Root)
        .unwrap_or_else(|| Payload::String("REDACTED".to_owned()));
    assert_eq!(result, serde_json::from_str::<Payload>(expected).unwrap());
    assert_eq!(input, serde_json::from_str::<Payload>(source).unwrap());
}
