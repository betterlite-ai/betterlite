#![cfg(test)]

use super::validate_values;
use crate::google_kms::GoogleKmsError;

#[test]
fn validate_environment_rejects_missing_google_credentials_or_resource() {
    assert!(matches!(
        validate_values(None, Some("resource")),
        Err(GoogleKmsError::MissingCredentials)
    ));
    assert!(matches!(
        validate_values(Some("credentials"), None),
        Err(GoogleKmsError::MissingResourceName)
    ));
}
