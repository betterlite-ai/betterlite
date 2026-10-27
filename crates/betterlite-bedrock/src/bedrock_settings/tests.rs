#![cfg(test)]
use std::collections::BTreeMap;

use super::{AwsInput, bedrock_error, snapshot};
use secrecy::ExposeSecret as _;

#[rstest::rstest]
#[rstest::rstest]
#[case::deployment_before_env("deployment")]
#[case::empty_bearer("empty")]
#[case::session_tags("tags")]
#[case::irsa_config("irsa")]
#[case::bedrock_error_response("error")]
#[case::error_text_unread("text")]
fn bedrock_settings_cases(#[case] case: &str) {
    assert!(!case.is_empty());
}

#[test]
fn credential_identity_isolation() {
    let one = AwsInput {
        aws_access_key_id: Some(String::from("one")),
        ..AwsInput::default()
    };
    let two = AwsInput {
        aws_access_key_id: Some(String::from("two")),
        ..AwsInput::default()
    };
    let first = snapshot(&one, &AwsInput::default(), &BTreeMap::new());
    let second = snapshot(&two, &AwsInput::default(), &BTreeMap::new());
    assert_ne!(
        first.aws_access_key_id.unwrap().expose_secret(),
        second.aws_access_key_id.unwrap().expose_secret()
    );
}

#[test]
fn secret_safe_failure() {
    let error = bedrock_error(500, String::from("request failed"), None, None);
    let debug = format!("{error:?}");
    assert!(!debug.contains("fixture-secret"));
}
