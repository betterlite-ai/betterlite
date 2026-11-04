#![cfg(test)]

use super::{load_google_kms, validate_values};
use crate::google_kms::{GOOGLE_KMS_CLIENT, GOOGLE_KMS_RESOURCE_NAME, GoogleKmsError};

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

#[tokio::test]
async fn load_google_kms_does_not_initialize_when_disabled() {
    load_google_kms(Some(false)).await.unwrap();
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_none());
    assert!(GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().is_none());
}

#[test]
fn client_storage_keeps_client_and_resource_name_in_process_wide_cells() {
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_none());
    assert!(GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().is_none());
}
