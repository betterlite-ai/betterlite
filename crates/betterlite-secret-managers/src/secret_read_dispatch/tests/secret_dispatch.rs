use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    ClientKind, assert_secret, aws_kms_client, azure_client, client_for, empty_environment,
    environment, google_kms_client, google_secrets_client, recorder,
};

const CIPHERTEXT_VALUE: &str = "dmFsdWU=";

#[rstest]
#[case::azure(ClientKind::Azure, Some(KeyManagementSystem::AzureKeyVault), "azure")]
#[case::google_kms(
    ClientKind::GoogleKms,
    Some(KeyManagementSystem::GoogleKms),
    "google_kms"
)]
#[case::aws_kms(ClientKind::AwsKms, Some(KeyManagementSystem::AwsKms), "aws_kms")]
#[case::aws_secrets(
    ClientKind::AwsSecretManager,
    Some(KeyManagementSystem::AwsSecretManager),
    "aws_secrets"
)]
#[case::google_secrets(
    ClientKind::GoogleSecretManager,
    Some(KeyManagementSystem::GoogleSecretManager),
    "google_secrets"
)]
#[case::vault(
    ClientKind::Vault,
    Some(KeyManagementSystem::HashicorpVault),
    "sync_reader"
)]
#[case::cyberark(
    ClientKind::CyberArk,
    Some(KeyManagementSystem::Cyberark),
    "sync_reader"
)]
#[case::custom(ClientKind::Custom, Some(KeyManagementSystem::Custom), "custom")]
#[case::infisical_default(ClientKind::Infisical, None, "infisical")]
fn each_key_manager_reaches_its_client(
    #[case] kind: ClientKind,
    #[case] system: Option<KeyManagementSystem>,
    #[case] expected_call: &'static str,
) {
    let recorder = recorder();
    let client = client_for(kind, &recorder);
    let result = get_secret_from_manager(
        &client,
        system,
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, "value");
    assert_eq!(recorder.calls(), [expected_call]);
}

#[test]
fn local_key_manager_reads_the_environment_without_a_client() {
    let result = get_secret_from_manager(
        &SecretManagerClient::Uninitialized,
        Some(KeyManagementSystem::Local),
        "secret-name",
        None,
        &environment("secret-name", "local-value"),
    );
    let secret = result
        .expect("a local read should succeed")
        .expect("the environment value should exist");
    assert_eq!(secret.expose_secret(), "local-value");
}

#[test]
fn azure_client_wins_over_another_key_manager() {
    let recorder = recorder();
    let client = azure_client(&recorder);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, "value");
    assert_eq!(recorder.calls(), ["azure"]);
}

#[test]
fn google_kms_client_wins_over_another_key_manager() {
    let recorder = recorder();
    let client = google_kms_client(&recorder);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, "value");
    assert_eq!(recorder.calls(), ["google_kms"]);
}

#[test]
fn azure_key_manager_requires_the_azure_client() {
    let recorder = recorder();
    let client = google_secrets_client(&recorder);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AzureKeyVault),
        "secret-name",
        None,
        &empty_environment(),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::WrongClient {
            manager: "Azure Key Vault",
            ..
        })
    ));
}

#[test]
fn google_kms_key_manager_requires_the_google_kms_client() {
    let recorder = recorder();
    let client = aws_kms_client(&recorder);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert!(matches!(
        result,
        Err(SecretDispatchError::WrongClient {
            manager: "Google KMS",
            ..
        })
    ));
}
