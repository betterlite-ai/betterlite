use std::sync::Arc;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    AwsKmsFake, AwsSecretsFake, AzureFake, ClientKind, CustomFake, GoogleKmsFake,
    GoogleSecretsFake, InfisicalFake, SyncReaderFake, assert_secret, aws_kms_fake,
    aws_secrets_fake, azure_fake, custom_fake, empty_environment, environment, google_kms_fake,
    google_secrets_fake, infisical_fake, sync_reader_fake,
};

const CIPHERTEXT_VALUE: &str = "dmFsdWU=";
const RESOURCE_NAME: &str = "projects/project/locations/global/keyRings/ring/cryptoKeys/key";

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
#[case::vault(ClientKind::Vault, Some(KeyManagementSystem::HashicorpVault), "vault")]
#[case::cyberark(ClientKind::CyberArk, Some(KeyManagementSystem::Cyberark), "cyberark")]
#[case::custom(ClientKind::Custom, Some(KeyManagementSystem::Custom), "custom")]
#[case::infisical_default(ClientKind::Infisical, None, "infisical")]
fn each_key_manager_reaches_its_client(
    #[case] kind: ClientKind,
    #[case] system: Option<KeyManagementSystem>,
    #[case] expected: &'static str,
) {
    let azure = azure_fake(Some("azure"));
    let google_kms = google_kms_fake(RESOURCE_NAME, Some(b"google_kms"));
    let aws_kms = aws_kms_fake(Some(b"aws_kms"));
    let aws_secrets = aws_secrets_fake(Some("aws_secrets"));
    let google_secrets = google_secrets_fake(Some("google_secrets"));
    let vault = sync_reader_fake(Some("vault"));
    let cyberark = sync_reader_fake(Some("cyberark"));
    let custom = custom_fake(Some("custom"));
    let infisical = infisical_fake(Some("infisical"));
    let client = match kind {
        ClientKind::Azure => SecretManagerClient::AzureKeyVault(Arc::<AzureFake>::clone(&azure)),
        ClientKind::GoogleKms => {
            SecretManagerClient::GoogleKms(Arc::<GoogleKmsFake>::clone(&google_kms))
        }
        ClientKind::AwsKms => SecretManagerClient::AwsKms(Arc::<AwsKmsFake>::clone(&aws_kms)),
        ClientKind::AwsSecretManager => {
            SecretManagerClient::AwsSecretManager(Arc::<AwsSecretsFake>::clone(&aws_secrets))
        }
        ClientKind::GoogleSecretManager => {
            SecretManagerClient::GoogleSecretManager(Arc::<GoogleSecretsFake>::clone(
                &google_secrets,
            ))
        }
        ClientKind::Vault => SecretManagerClient::Vault(Arc::<SyncReaderFake>::clone(&vault)),
        ClientKind::CyberArk => {
            SecretManagerClient::CyberArk(Arc::<SyncReaderFake>::clone(&cyberark))
        }
        ClientKind::Custom => SecretManagerClient::Custom(Arc::<CustomFake>::clone(&custom)),
        ClientKind::Infisical => {
            SecretManagerClient::Infisical(Arc::<InfisicalFake>::clone(&infisical))
        }
    };
    let result = get_secret_from_manager(
        &client,
        system,
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, expected);
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
    let fake = azure_fake(Some("azure"));
    let client = SecretManagerClient::AzureKeyVault(Arc::<AzureFake>::clone(&fake));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, "azure");
}

#[test]
fn google_kms_client_wins_over_another_key_manager() {
    let fake = google_kms_fake(RESOURCE_NAME, Some(b"google_kms"));
    let client = SecretManagerClient::GoogleKms(Arc::<GoogleKmsFake>::clone(&fake));
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "secret-name",
        None,
        &environment("secret-name", CIPHERTEXT_VALUE),
    );
    assert_secret(result, "google_kms");
}

#[test]
fn azure_key_manager_requires_the_azure_client() {
    let fake = google_secrets_fake(Some("value"));
    let client = SecretManagerClient::GoogleSecretManager(Arc::<GoogleSecretsFake>::clone(&fake));
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
    let fake = aws_kms_fake(Some(b"plain"));
    let client = SecretManagerClient::AwsKms(Arc::<AwsKmsFake>::clone(&fake));
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
