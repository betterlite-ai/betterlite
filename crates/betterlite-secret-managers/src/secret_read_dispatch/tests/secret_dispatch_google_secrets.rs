use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, empty_environment, google_secrets_client, google_secrets_failing_fake,
    google_secrets_fake,
};

#[test]
fn google_secrets_returns_the_secret() {
    let fake = google_secrets_fake(Some("gsm-value"));
    let client = google_secrets_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleSecretManager),
        "db-password",
        None,
        &empty_environment(),
    );
    assert_secret(result, "gsm-value");
    assert_eq!(fake.names(), ["db-password"]);
}

#[test]
fn google_secrets_client_failure_propagates() {
    let client = google_secrets_client(&google_secrets_failing_fake());
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleSecretManager),
        "db-password",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::GoogleSecretManager(_))
    ));
}
