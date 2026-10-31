#![cfg(test)]

use super::RedisSettingsError;

#[test]
fn missing_target_matches_litellms_error_message() {
    assert_eq!(
        RedisSettingsError::MissingTarget.to_string(),
        "Either 'host' or 'url' must be specified for redis."
    );
}

#[test]
fn aws_iam_requires_tls_matches_litellms_error_message() {
    assert_eq!(
        RedisSettingsError::AwsIamRequiresTls.to_string(),
        "AWS ElastiCache IAM Redis authentication requires TLS"
    );
}

#[test]
fn aws_iam_missing_settings_lists_every_missing_name() {
    assert_eq!(
        RedisSettingsError::AwsIamMissingSettings {
            missing: vec!["aws_iam_user_name".to_owned(), "aws_iam_region".to_owned()],
        }
        .to_string(),
        "AWS ElastiCache IAM Redis authentication requires: aws_iam_user_name, aws_iam_region"
    );
}

#[test]
fn unknown_option_names_the_rejected_key() {
    assert_eq!(
        RedisSettingsError::UnknownOption {
            name: "not_a_real_option".to_owned(),
        }
        .to_string(),
        "redis connection option 'not_a_real_option' is not recognized"
    );
}
