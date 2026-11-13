use crate::custom_interface::CustomParams;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, aws_secrets_client, aws_secrets_failing_fake, aws_secrets_fake,
    empty_environment,
};

#[test]
fn aws_secrets_receives_the_primary_secret_name_from_settings() {
    let fake = aws_secrets_fake(Some("primary-value"));
    let client = aws_secrets_client(&fake);
    let settings = SecretManagerSettings {
        primary_secret_name: Some("primary".to_owned()),
        optional_params: CustomParams::new(),
    };
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsSecretManager),
        "db",
        Some(&settings),
        &empty_environment(),
    );
    assert_secret(result, "primary-value");
    assert_eq!(
        fake.calls(),
        [("db".to_owned(), Some("primary".to_owned()))]
    );
}

#[test]
fn aws_secrets_without_settings_passes_no_primary_name() {
    let fake = aws_secrets_fake(Some("direct-value"));
    let client = aws_secrets_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsSecretManager),
        "db",
        None,
        &empty_environment(),
    );
    assert_secret(result, "direct-value");
    assert_eq!(fake.calls(), [("db".to_owned(), None)]);
}

#[test]
fn aws_secrets_read_failure_propagates() {
    let client = aws_secrets_client(&aws_secrets_failing_fake());
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsSecretManager),
        "db",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::AwsSecretManager(_))
    ));
}
