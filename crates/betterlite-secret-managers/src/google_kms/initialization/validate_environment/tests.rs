#![cfg(test)]

use super::validate_environment;
use crate::google_kms::GoogleKmsError;
use std::collections::BTreeMap;

#[test]
fn validate_environment_rejects_missing_google_credentials_or_resource() {
    assert!(matches!(
        validate_environment(&BTreeMap::new()),
        Err(GoogleKmsError::MissingCredentials)
    ));
    let environment = BTreeMap::from([(
        "GOOGLE_APPLICATION_CREDENTIALS".to_owned(),
        "credentials".to_owned(),
    )]);
    assert!(matches!(
        validate_environment(&environment),
        Err(GoogleKmsError::MissingResourceName)
    ));
}
