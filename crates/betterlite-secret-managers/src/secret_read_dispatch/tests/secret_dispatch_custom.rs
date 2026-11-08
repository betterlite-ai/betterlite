use crate::custom_interface::CustomParams;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, custom_client, custom_failing_fake, custom_fake, custom_optional_params,
    empty_environment, google_secrets_client, google_secrets_fake,
};

#[test]
fn custom_receives_the_optional_params_from_settings() {
    let fake = custom_fake(Some("custom-value"));
    let client = custom_client(&fake);
    let settings = SecretManagerSettings {
        primary_secret_name: None,
        optional_params: custom_optional_params(),
    };
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Custom),
        "db",
        Some(&settings),
        &empty_environment(),
    );
    assert_secret(result, "custom-value");
    assert_eq!(
        fake.calls(),
        [("db".to_owned(), Some(custom_optional_params()))]
    );
}

#[test]
fn custom_without_settings_passes_no_params() {
    let fake = custom_fake(Some("custom-value"));
    let client = custom_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Custom),
        "db",
        None,
        &empty_environment(),
    );
    assert_secret(result, "custom-value");
    assert_eq!(
        fake.calls(),
        [("db".to_owned(), Option::<CustomParams>::None)]
    );
}

#[test]
fn custom_wrong_client_type_names_the_client() {
    let client = google_secrets_client(&google_secrets_fake(Some("value")));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Custom),
        "db",
        None,
        &empty_environment(),
    );
    let Err(error) = result else {
        panic!("a foreign client should be rejected");
    };
    assert!(error.to_string().contains("got GoogleSecretManager"));
    assert!(matches!(
        error,
        SecretDispatchError::CustomWrongClient {
            actual: "GoogleSecretManager"
        }
    ));
}

#[test]
fn custom_client_failure_propagates() {
    let client = custom_client(&custom_failing_fake());
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Custom),
        "db",
        None,
        &empty_environment(),
    );
    assert!(matches!(result, Err(SecretDispatchError::Custom(_))));
}
