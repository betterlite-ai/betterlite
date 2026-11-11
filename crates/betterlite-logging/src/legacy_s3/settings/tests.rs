#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{SettingsError, resolve};

fn params(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn reads_the_connection_keys() {
    let settings = resolve(&params(&[
        ("s3_bucket_name", "logs"),
        ("s3_path", "team-logs"),
        ("s3_region_name", "eu-west-1"),
        ("s3_endpoint_url", "https://s3.example.com"),
    ]))
    .expect("the settings resolve");
    assert_eq!(settings.bucket_name.as_deref(), Some("logs"));
    assert_eq!(settings.path, "team-logs");
    assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(
        settings.endpoint_url.as_deref(),
        Some("https://s3.example.com")
    );
}

#[test]
fn reads_the_credentials_and_flags() {
    let settings = resolve(&params(&[
        ("s3_aws_access_key_id", "access"),
        ("s3_aws_secret_access_key", "secret"),
        ("s3_aws_session_token", "token"),
        ("s3_server_side_encryption", "aws:kms"),
        ("s3_sse_kms_key_id", "key-1"),
        ("s3_use_team_prefix", "true"),
        ("s3_log_prompts_only", "false"),
        ("enable_preview_features", "yes"),
    ]))
    .expect("the settings resolve");
    assert_eq!(
        settings
            .access_key_id
            .expect("the key is set")
            .expose_secret(),
        "access"
    );
    assert_eq!(
        settings
            .secret_access_key
            .expect("the secret is set")
            .expose_secret(),
        "secret"
    );
    assert_eq!(
        settings
            .session_token
            .expect("the token is set")
            .expose_secret(),
        "token"
    );
    let encryption = settings.encryption.expect("encryption is set");
    assert_eq!(encryption.algorithm, "aws:kms");
    assert_eq!(encryption.kms_key_id.as_deref(), Some("key-1"));
    assert!(settings.use_team_prefix);
    assert!(settings.preview_features);
    assert!(settings.log_prompts_only.is_some());
}

#[test]
fn leaves_every_optional_key_unset() {
    let settings = resolve(&params(&[])).expect("the settings resolve");
    assert!(settings.bucket_name.is_none());
    assert!(settings.path.is_empty());
    assert!(settings.region_name.is_none());
    assert!(settings.endpoint_url.is_none());
    assert!(settings.access_key_id.is_none());
    assert!(settings.secret_access_key.is_none());
    assert!(settings.session_token.is_none());
    assert!(settings.encryption.is_none());
    assert!(!settings.use_team_prefix);
    assert!(!settings.preview_features);
    assert!(settings.log_prompts_only.is_none());
}

#[rstest]
#[case::api_version("s3_api_version")]
#[case::use_ssl("s3_use_ssl")]
#[case::verify("s3_verify")]
#[case::config("s3_config")]
#[case::profile("s3_aws_profile_name")]
fn rejects_each_unsupported_key(#[case] name: &str) {
    let error = resolve(&params(&[(name, "value")])).expect_err("the key is refused");
    assert!(matches!(error, SettingsError::UnknownKey { name: refused } if refused == name));
}

#[rstest]
#[case::true_word("true", true)]
#[case::capitalised("True", true)]
#[case::yes_word("yes", true)]
#[case::one("1", true)]
#[case::off_word("off", false)]
#[case::zero("0", false)]
#[case::false_word("false", false)]
fn parses_the_boolean_flags(#[case] word: &str, #[case] expected: bool) {
    let settings = resolve(&params(&[
        ("s3_use_team_prefix", word),
        ("enable_preview_features", word),
    ]))
    .expect("the settings resolve");
    assert_eq!(settings.use_team_prefix, expected);
    assert_eq!(settings.preview_features, expected);
}

#[test]
fn refuses_a_flag_that_is_not_a_boolean() {
    let error =
        resolve(&params(&[("s3_use_team_prefix", "banana")])).expect_err("the flag is refused");
    assert!(matches!(
        error,
        SettingsError::Boolean {
            name: "s3_use_team_prefix",
            ..
        }
    ));
}

#[test]
fn a_kms_key_alone_selects_kms_encryption() {
    let settings =
        resolve(&params(&[("s3_sse_kms_key_id", "key-1")])).expect("the settings resolve");
    let encryption = settings.encryption.expect("encryption is set");
    assert_eq!(encryption.algorithm, "aws:kms");
    assert_eq!(encryption.kms_key_id.as_deref(), Some("key-1"));
}

#[test]
fn a_non_kms_algorithm_drops_the_kms_key() {
    let settings = resolve(&params(&[
        ("s3_server_side_encryption", "AES256"),
        ("s3_sse_kms_key_id", "key-1"),
    ]))
    .expect("the settings resolve");
    let encryption = settings.encryption.expect("encryption is set");
    assert_eq!(encryption.algorithm, "AES256");
    assert!(encryption.kms_key_id.is_none());
}
