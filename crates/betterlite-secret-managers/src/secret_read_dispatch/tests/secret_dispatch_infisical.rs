use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;

use super::support::{
    assert_secret, empty_environment, infisical_client, infisical_failing_fake, infisical_fake,
};

#[test]
fn unknown_key_manager_falls_back_to_infisical() {
    let fake = infisical_fake(Some("infisical-value"));
    let client = infisical_client(&fake);
    let result = get_secret_from_manager(&client, None, "db-password", None, &empty_environment());
    assert_secret(result, "infisical-value");
    assert_eq!(fake.names(), ["db-password"]);
}

#[test]
fn unknown_key_manager_without_an_infisical_client_is_an_error() {
    let result = get_secret_from_manager(
        &SecretManagerClient::Uninitialized,
        None,
        "db-password",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::WrongClient {
            manager: "Infisical",
            ..
        })
    ));
}

#[test]
fn infisical_client_failure_propagates() {
    let client = infisical_client(&infisical_failing_fake());
    let result = get_secret_from_manager(&client, None, "db-password", None, &empty_environment());
    assert!(matches!(result, Err(SecretDispatchError::Infisical(_))));
}
