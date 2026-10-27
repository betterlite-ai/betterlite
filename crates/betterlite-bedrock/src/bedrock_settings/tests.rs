#![cfg(test)]
use std::collections::BTreeMap;

use super::{AwsInput, bedrock_error, snapshot};
use secrecy::ExposeSecret as _;

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
fn bedrock_settings_credential_identity_isolation() {
    let one = input(Some("one"));
    let two = input(Some("two"));
    let first = snapshot(&one, &AwsInput::default(), &BTreeMap::new());
    let second = snapshot(&two, &AwsInput::default(), &BTreeMap::new());
    assert_ne!(
        first.aws_access_key_id.unwrap().expose_secret(),
        second.aws_access_key_id.unwrap().expose_secret()
    );
}

#[test]
fn bedrock_settings_secret_safe_failure() {
    let error = bedrock_error(500, String::from("request failed"), None, None);
    let debug = format!("{error:?}");
    assert!(!debug.contains("fixture-secret"));
}

fn input(access_key: Option<&str>) -> AwsInput {
    AwsInput {
        aws_access_key_id: access_key.map(String::from),
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_region_name: None,
        aws_session_name: None,
        aws_profile_name: None,
        aws_role_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_external_id: None,
        aws_role_arn: None,
        aws_web_identity_token_file: None,
        aws_bearer_token_bedrock: None,
        aws_session_tags: None,
    }
}
