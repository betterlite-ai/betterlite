use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::value::RawValue;

use crate::agent_identity::legacy::{
    LEGACY_IDENTITY_MESSAGE, LEGACY_IDENTITY_STATUS, has_legacy_identity, reject_legacy_identity,
};

fn params(text: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::identity_object(r#"{"identity":{"provider":"microsoft_entra"}}"#, true)]
#[case::identity_null(r#"{"identity":null}"#, true)]
#[case::other_keys(r#"{"make_public":true}"#, false)]
#[case::empty_object("{}", false)]
fn legacy_identity_detected_from_litellm_params(#[case] input: &str, #[case] expected: bool) {
    assert_eq!(has_legacy_identity(Some(&params(input))), expected);
}

#[test]
fn legacy_identity_absent_params_are_allowed() {
    assert!(!has_legacy_identity::<Box<RawValue>>(None));
    reject_legacy_identity::<Box<RawValue>>(None).unwrap();
}

#[rstest]
#[case::identity_null(r#"{"identity":null}"#)]
#[case::identity_object(r#"{"identity":{"provider":"microsoft_entra"}}"#)]
fn legacy_identity_rejected_with_bad_request(#[case] input: &str) {
    let error = reject_legacy_identity(Some(&params(input))).unwrap_err();
    assert_eq!(error.status, LEGACY_IDENTITY_STATUS);
    assert_eq!(error.message, LEGACY_IDENTITY_MESSAGE);
}

#[test]
fn legacy_identity_rejection_allows_other_params() {
    reject_legacy_identity(Some(&params(r#"{"make_public":true}"#))).unwrap();
}
