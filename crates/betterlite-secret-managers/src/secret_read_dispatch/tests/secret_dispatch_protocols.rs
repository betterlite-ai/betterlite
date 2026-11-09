use std::error::Error;

use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::error::SecretDispatchError;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::KeyManagementSystem;

use super::support::{
    assert_secret, aws_kms_client, aws_kms_fake, aws_secrets_client, aws_secrets_failing_fake,
    aws_secrets_fake, azure_client, azure_failing_fake, azure_fake, base64_of, custom_client,
    custom_fake, custom_optional_params, cyberark_client, empty_environment, environment,
    google_kms_client, google_kms_fake, google_secrets_client, google_secrets_fake,
    infisical_client, infisical_failing_fake, infisical_fake, sync_reader_fake, vault_client,
};

fn deepest_source(error: &SecretDispatchError) -> String {
    let mut source: &dyn Error = error;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

#[test]
fn protocol_client_failures_keep_their_source() {
    let azure = azure_client(&azure_failing_fake());
    let azure_error = get_secret_from_manager(
        &azure,
        Some(KeyManagementSystem::AzureKeyVault),
        "db",
        None,
        &empty_environment(),
    )
    .expect_err("an Azure failure should raise");
    assert!(deepest_source(&azure_error).contains("azure transport failed"));

    let aws_secrets = aws_secrets_client(&aws_secrets_failing_fake());
    let aws_error = get_secret_from_manager(
        &aws_secrets,
        Some(KeyManagementSystem::AwsSecretManager),
        "db",
        None,
        &empty_environment(),
    )
    .expect_err("an AWS Secrets Manager failure should raise");
    assert!(deepest_source(&aws_error).contains("aws secrets transport failed"));

    let infisical = infisical_client(&infisical_failing_fake());
    let infisical_error =
        get_secret_from_manager(&infisical, None, "db", None, &empty_environment())
            .expect_err("an Infisical failure should raise");
    assert!(deepest_source(&infisical_error).contains("infisical transport failed"));
}

#[test]
fn azure_key_vault_client_exposes_the_secret_value() {
    let fake = azure_fake(Some("azure-value"));
    let client = azure_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AzureKeyVault),
        "db",
        None,
        &empty_environment(),
    );
    assert_secret(result, "azure-value");
}

#[test]
fn google_kms_client_carries_the_resource_name() {
    let fake = google_kms_fake(
        "projects/project/locations/global/keyRings/ring/cryptoKeys/key",
        Some(b"plain"),
    );
    let client = google_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("plain")),
    );
    assert_secret(result, "plain");
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].0,
        "projects/project/locations/global/keyRings/ring/cryptoKeys/key"
    );
}

#[test]
fn aws_kms_client_receives_the_ciphertext_blob() {
    let fake = aws_kms_fake(Some(b"plain"));
    let client = aws_kms_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::AwsKms),
        "encrypted",
        None,
        &environment("encrypted", &base64_of("plain")),
    );
    assert_secret(result, "plain");
    assert_eq!(fake.blobs(), [b"plain".to_vec()]);
}

#[test]
fn google_secret_manager_client_is_called_by_its_exact_name() {
    let fake = google_secrets_fake(Some("gsm-value"));
    let client = google_secrets_client(&fake);
    let result = get_secret_from_manager(
        &client,
        Some(KeyManagementSystem::GoogleSecretManager),
        "db",
        None,
        &empty_environment(),
    );
    assert_secret(result, "gsm-value");
    assert_eq!(fake.names(), ["db"]);
}

#[test]
fn sync_secret_reader_serves_vault_and_cyberark() {
    let fake = sync_reader_fake(Some("sync-value"));
    let vault = vault_client(&fake);
    let cyberark = cyberark_client(&fake);
    assert_secret(
        get_secret_from_manager(
            &vault,
            Some(KeyManagementSystem::HashicorpVault),
            "first",
            None,
            &empty_environment(),
        ),
        "sync-value",
    );
    assert_secret(
        get_secret_from_manager(
            &cyberark,
            Some(KeyManagementSystem::Cyberark),
            "second",
            None,
            &empty_environment(),
        ),
        "sync-value",
    );
    assert_eq!(fake.names(), ["first", "second"]);
}

#[test]
fn custom_secret_manager_client_receives_optional_params() {
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
fn infisical_client_exposes_the_secret_value() {
    let fake = infisical_fake(Some("infisical-value"));
    let client = infisical_client(&fake);
    let result = get_secret_from_manager(&client, None, "db", None, &empty_environment());
    assert_secret(result, "infisical-value");
}

#[test]
fn every_protocol_client_fits_the_handle_and_names_itself() {
    let handles = [
        azure_client(&azure_fake(None)),
        google_kms_client(&google_kms_fake("resource", None)),
        aws_kms_client(&aws_kms_fake(None)),
        aws_secrets_client(&aws_secrets_fake(None)),
        google_secrets_client(&google_secrets_fake(None)),
        vault_client(&sync_reader_fake(None)),
        cyberark_client(&sync_reader_fake(None)),
        custom_client(&custom_fake(None)),
        infisical_client(&infisical_fake(None)),
        SecretManagerClient::Uninitialized,
    ];
    let names = handles
        .iter()
        .map(SecretManagerClient::class_name)
        .collect::<Vec<&str>>();
    assert_eq!(
        names,
        [
            "SecretClient",
            "KeyManagementServiceClient",
            "AWSKeyManagementService",
            "AWSSecretsManagerV2",
            "GoogleSecretManager",
            "HashicorpSecretManager",
            "CyberArkSecretManager",
            "CustomSecretManager",
            "InfisicalSecretManager",
            "NoneType",
        ]
    );
}
