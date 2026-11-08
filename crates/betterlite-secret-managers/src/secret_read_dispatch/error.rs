use std::str::Utf8Error;

use thiserror::Error;

use super::protocols::{AwsSecretsManagerError, AzureKeyVaultError, InfisicalError};
use crate::aws_kms::AwsKmsError;
use crate::custom_interface::CustomSecretError;
use crate::google_kms::GoogleKmsError;
use crate::google_secret_manager::GoogleSecretManagerError;

#[derive(Debug, Error)]
pub enum SecretDispatchError {
    #[error("Google KMS requires the encrypted secret to be in the environment!")]
    GoogleKmsMissingSecret,
    #[error("Google KMS requires the encrypted secret to be encoded in base64")]
    GoogleKmsInvalidBase64,
    #[error("Google KMS ciphertext is not valid base64")]
    GoogleKmsBase64(#[source] base64::DecodeError),
    #[error("Google KMS decryption failed")]
    GoogleKms(#[source] GoogleKmsError),
    #[error("Google KMS plaintext is not UTF-8")]
    GoogleKmsUtf8(#[source] Utf8Error),
    #[error("AWS KMS ciphertext is not valid base64")]
    AwsKmsBase64(#[source] base64::DecodeError),
    #[error("AWS KMS decryption failed")]
    AwsKms(#[source] AwsKmsError),
    #[error("AWS KMS plaintext is not UTF-8")]
    AwsKmsUtf8(#[source] Utf8Error),
    #[error("AWS Secrets Manager read failed")]
    AwsSecretManager(#[source] AwsSecretsManagerError),
    #[error("Google Secret Manager read failed")]
    GoogleSecretManager(#[source] GoogleSecretManagerError),
    #[error("No secret found in Google Secret Manager for {secret_name}")]
    GoogleSecretManagerMissing { secret_name: String },
    #[error("No secret found in Hashicorp Secret Manager for {secret_name}")]
    VaultMissing { secret_name: String },
    #[error("No secret found in CyberArk Secret Manager for {secret_name}")]
    CyberArkMissing { secret_name: String },
    #[error("Custom Secret Manager read failed")]
    Custom(#[source] CustomSecretError),
    #[error("No secret found in Custom Secret Manager for {secret_name}")]
    CustomMissing { secret_name: String },
    #[error(
        "Custom secret manager client must be an instance of CustomSecretManager, got {actual}"
    )]
    CustomWrongClient { actual: &'static str },
    #[error("Azure Key Vault secret read failed")]
    AzureKeyVault(#[source] AzureKeyVaultError),
    #[error("Infisical secret read failed")]
    Infisical(#[source] InfisicalError),
    #[error("the {expected} client is required for {manager}, got {actual}")]
    WrongClient {
        manager: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
}
