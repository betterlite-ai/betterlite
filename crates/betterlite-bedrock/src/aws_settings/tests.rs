#![cfg(test)]

use super::tls::resolve_tls;
use betterlite_config::{
    loader::ConfigLoader,
    tls::{Verification, VerifySetting},
};
use std::collections::BTreeMap;

mod aws_settings_cases;
mod metadata;
mod support;

#[test]
fn invalid_tags_do_not_echo_secrets() {
    let secret = "fixture-secret-never-log";
    let error = super::canonicalize_tags(Some(
        serde_json::from_str(&format!(r#"[{{"Key":"{secret}","Value":42}}]"#)).unwrap(),
    ))
    .unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}

#[rstest::rstest]
#[case::disabled(VerifySetting::Bool(false), Verification::Disabled)]
#[case::enabled(VerifySetting::Bool(true), Verification::Enabled)]
#[case::custom_ca(VerifySetting::Path("/fixture-ca.pem".to_owned()), Verification::CaFile("/fixture-ca.pem".to_owned()))]
fn delegates_tls(#[case] explicit: VerifySetting, #[case] expected: Verification) {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .unwrap();
    let policy = resolve_tls(&config, &environment, Some(&explicit)).unwrap();
    assert_eq!(policy.verification, expected);
}

#[test]
fn popped_tags_are_canonical_and_extras_are_ignored() {
    let mut params = serde_json::from_str(r#"{"aws_session_tags":[{"Key":"z","Value":"2","Extra":42},{"Key":"a","Value":"1"}],"other":true}"#).unwrap();
    let auth = super::pop_aws_auth_params(&mut params).unwrap();
    assert_eq!(
        auth.aws_session_tags
            .unwrap()
            .iter()
            .map(|tag| tag.key.as_str())
            .collect::<Vec<_>>(),
        ["a", "z"]
    );
    assert_eq!(params.len(), 1);
    assert!(params.contains_key("other"));
    assert_eq!(super::AUTHENTICATION_PARAMS.len(), 12);
    assert_eq!(super::AUTHENTICATION_PARAMS[3], "aws_region_name");
}
