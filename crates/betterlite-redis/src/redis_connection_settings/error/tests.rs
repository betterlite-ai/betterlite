#![cfg(test)]

use super::RedisSettingsError;

#[test]
fn missing_target_matches_litellms_error_message() {
    assert_eq!(
        RedisSettingsError::MissingTarget.to_string(),
        "Either 'host' or 'url' must be specified for redis."
    );
}
