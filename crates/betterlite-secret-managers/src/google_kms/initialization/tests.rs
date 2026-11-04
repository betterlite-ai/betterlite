#![cfg(test)]

use super::{build_client, load_google_kms, store_client, validate_values};
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
}

#[tokio::test]
async fn load_google_kms_constructs_the_google_sdk_client() {
    let _client = build_client().await.unwrap();
}

#[tokio::test]
async fn client_storage_keeps_client_and_resource_name_in_process_wide_cells() {
    let client = build_client().await.unwrap();
    store_client(
        client,
        "projects/p/locations/l/keyRings/r/cryptoKeys/k".to_owned(),
    )
    .unwrap();
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_some());
    assert_eq!(
        GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().as_deref(),
        Some("projects/p/locations/l/keyRings/r/cryptoKeys/k")
    );
}
