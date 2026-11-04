#![cfg(test)]

use super::{KeyManagementSystem, SecretAccessMode};
use rstest::rstest;

#[rstest]
#[case::aws_kms(KeyManagementSystem::AwsKms, "aws_kms")]
#[case::aws_secret_manager(KeyManagementSystem::AwsSecretManager, "aws_secret_manager")]
#[case::google_secret_manager(KeyManagementSystem::GoogleSecretManager, "google_secret_manager")]
#[case::google_kms(KeyManagementSystem::GoogleKms, "google_kms")]
#[case::hashicorp_vault(KeyManagementSystem::HashicorpVault, "hashicorp_vault")]
#[case::cyberark(KeyManagementSystem::Cyberark, "cyberark")]
#[case::azure_key_vault(KeyManagementSystem::AzureKeyVault, "azure_key_vault")]
#[case::local(KeyManagementSystem::Local, "local")]
#[case::custom(KeyManagementSystem::Custom, "custom")]
fn secret_manager_discriminants_match_litellm_values(
    #[case] variant: KeyManagementSystem,
    #[case] expected: &str,
) {
    assert_eq!(
        serde_json::to_string(&variant).ok().as_deref(),
        Some(format!("\"{expected}\"").as_str())
    );
}

#[test]
fn access_modes_use_expected_strings() {
    assert_eq!(
        serde_json::to_string(&SecretAccessMode::ReadOnly)
            .ok()
            .as_deref(),
        Some("\"read_only\"")
    );
    assert_eq!(
        serde_json::to_string(&SecretAccessMode::WriteOnly)
            .ok()
            .as_deref(),
        Some("\"write_only\"")
    );
    assert_eq!(
        serde_json::to_string(&SecretAccessMode::ReadAndWrite)
            .ok()
            .as_deref(),
        Some("\"read_and_write\"")
    );
}
