use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{assert_secret, aws_kms_client, aws_kms_fake, empty_environment, environment};

#[test]
fn local_reads_the_environment_value() {
    let result = get_secret_from_manager(
        &SecretManagerClient::Uninitialized,
        Some(KeyManagementSystem::Local),
        "db-password",
        None,
        &environment("db-password", "local-value"),
    );
    assert_secret(result, "local-value");
}

#[test]
fn local_leaves_a_non_matching_client_alone() {
    let fake = aws_kms_fake(Some(b"unused"));
    let client = aws_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::Local),
        "db-password",
        None,
        &environment("db-password", "local-value"),
    );
    assert_secret(result, "local-value");
    assert!(fake.blobs().is_empty());
}

#[test]
fn local_does_not_read_a_missing_name() {
    let result = get_secret_from_manager(
        &SecretManagerClient::Uninitialized,
        Some(KeyManagementSystem::Local),
        "db-password",
        None,
        &empty_environment(),
    );
    assert!(matches!(result, Ok(None)));
}
