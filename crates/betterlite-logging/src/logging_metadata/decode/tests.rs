#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde::Deserialize;

use super::DecodeError;
use super::decode;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Sample {
    count: i64,
    label: Option<String>,
}

fn fields(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("fields decode")
}

#[rstest]
#[case::all_fields(r#"{"count": 3, "label": "a"}"#)]
#[case::optional_absent(r#"{"count": 3}"#)]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str) {
    let decoded: Sample = decode("sample", fields(encoded)).expect("record decodes");
    assert_eq!(decoded.count, 3);
}

#[test]
fn a_mistyped_field_names_the_record_and_the_path() {
    let error =
        decode::<Sample>("sample", fields(r#"{"count": "three"}"#)).expect_err("wrong type");
    let DecodeError::Field { record, path, .. } = error else {
        panic!("a field error is expected");
    };
    assert_eq!(record, "sample");
    assert_eq!(path, "count");
}

#[test]
fn an_unknown_field_is_refused() {
    let error = decode::<Sample>("sample", fields(r#"{"count": 1, "extra": 2}"#))
        .expect_err("unknown field");
    assert!(matches!(error, DecodeError::Field { .. }));
}
