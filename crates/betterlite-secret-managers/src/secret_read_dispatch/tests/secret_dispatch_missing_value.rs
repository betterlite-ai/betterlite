use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_no_secret, aws_kms_client, aws_kms_fake, azure_client, azure_fake, custom_client,
    custom_fake, cyberark_client, empty_environment, google_secrets_client, google_secrets_fake,
    infisical_client, infisical_fake, sync_reader_fake, vault_client,
};

#[test]
fn aws_kms_missing_environment_value_returns_none() {
    let client = aws_kms_client(&aws_kms_fake(Some(b"plain")));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "missing",
        None,
        &empty_environment(),
    );
    assert_no_secret(&result);
}

#[test]
fn aws_secret_manager_type_mismatch_returns_none() {
    let fake = sync_reader_fake(Some("unused"));
    let client = vault_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsSecretManager),
        "db",
        None,
        &empty_environment(),
    );
    assert_no_secret(&result);
    assert!(fake.names().is_empty());
}

#[test]
fn google_secret_manager_none_raises() {
    let client = google_secrets_client(&google_secrets_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleSecretManager),
        "db",
        None,
        &empty_environment(),
    );
    let Err(SecretDispatchError::GoogleSecretManagerMissing { secret_name }) = result else {
        panic!("a missing google secret should raise");
    };
    assert_eq!(secret_name, "db");
}

#[test]
fn vault_none_raises() {
    let client = vault_client(&sync_reader_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::HashicorpVault),
        "db",
        None,
        &empty_environment(),
    );
    let Err(SecretDispatchError::VaultMissing { secret_name }) = result else {
        panic!("a missing vault secret should raise");
    };
    assert_eq!(secret_name, "db");
}

#[test]
fn cyberark_none_raises() {
    let client = cyberark_client(&sync_reader_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Cyberark),
        "db",
        None,
        &empty_environment(),
    );
    let Err(SecretDispatchError::CyberArkMissing { secret_name }) = result else {
        panic!("a missing cyberark secret should raise");
    };
    assert_eq!(secret_name, "db");
}

#[test]
fn custom_none_raises() {
    let client = custom_client(&custom_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Custom),
        "db",
        None,
        &empty_environment(),
    );
    let Err(SecretDispatchError::CustomMissing { secret_name }) = result else {
        panic!("a missing custom secret should raise");
    };
    assert_eq!(secret_name, "db");
}

#[test]
fn azure_none_value_returns_none() {
    let client = azure_client(&azure_fake(None));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AzureKeyVault),
        "db",
        None,
        &empty_environment(),
    );
    assert_no_secret(&result);
}

#[test]
fn infisical_none_value_returns_none() {
    let client = infisical_client(&infisical_fake(None));
    let result = get_secret_from_manager(&client, None, "db", None, &empty_environment());
    assert_no_secret(&result);
}

#[test]
fn local_missing_value_returns_none() {
    let result = get_secret_from_manager(
        &SecretManagerClient::Uninitialized,
        Some(KeyManagementSystem::Local),
        "db",
        None,
        &empty_environment(),
    );
    assert_no_secret(&result);
}
