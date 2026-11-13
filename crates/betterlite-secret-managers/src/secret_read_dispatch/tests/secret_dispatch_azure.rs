use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, azure_client, azure_failing_fake, azure_fake, empty_environment,
};

#[test]
fn azure_returns_the_secret_value() {
    let fake = azure_fake(Some("azure-value"));
    let client = azure_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AzureKeyVault),
        "db-password",
        None,
        &empty_environment(),
    );
    assert_secret(result, "azure-value");
    assert_eq!(fake.names(), ["db-password"]);
}

#[test]
fn azure_client_failure_propagates() {
    let client = azure_client(&azure_failing_fake());
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AzureKeyVault),
        "db-password",
        None,
        &empty_environment(),
    );
    assert!(matches!(result, Err(SecretDispatchError::AzureKeyVault(_))));
}
