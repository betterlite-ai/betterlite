use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, aws_kms_client, aws_kms_fake, base64_of, empty_environment, environment,
    google_kms_client, google_kms_fake,
};

const RESOURCE_NAME: &str = "projects/project/locations/global/keyRings/ring/cryptoKeys/key";

#[test]
fn google_kms_decrypts_the_environment_ciphertext() {
    let fake = google_kms_fake(RESOURCE_NAME, Some(b"plain-text"));
    let client = google_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("plain-text")),
    );
    assert_secret(result, "plain-text");
    assert_eq!(
        fake.requests(),
        [(RESOURCE_NAME.to_owned(), b"plain-text".to_vec())]
    );
}

#[test]
fn google_kms_requires_the_environment_value() {
    let client = google_kms_client(&google_kms_fake(RESOURCE_NAME, Some(b"plain")));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "missing",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::GoogleKmsMissingSecret)
    ));
}

#[test]
fn google_kms_checks_the_environment_before_the_client_type() {
    let client = aws_kms_client(&aws_kms_fake(Some(b"plain")));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "missing",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::GoogleKmsMissingSecret)
    ));
}

#[test]
fn google_kms_rejects_non_canonical_base64() {
    let fake = google_kms_fake(RESOURCE_NAME, Some(b"plain"));
    let client = google_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "encrypted",
        None,
        &environment("encrypted", "YQ"),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::GoogleKmsInvalidBase64)
    ));
    assert!(fake.requests().is_empty());
}

#[test]
fn google_kms_plaintext_must_be_utf8() {
    let client = google_kms_client(&google_kms_fake(RESOURCE_NAME, Some(&[0xff])));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("whatever")),
    );
    assert!(matches!(result, Err(SecretDispatchError::GoogleKmsUtf8(_))));
}

#[test]
fn google_kms_client_failure_propagates() {
    let client = google_kms_client(&google_kms_fake(RESOURCE_NAME, None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("plain")),
    );
    assert!(matches!(result, Err(SecretDispatchError::GoogleKms(_))));
}
