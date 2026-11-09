use std::io;

use secrecy::SecretString;

use crate::aws_kms::AwsKmsError;
use crate::google_kms::GoogleKmsError;
use crate::google_secret_manager::GoogleSecretManagerError;

#[derive(Debug)]
pub struct AzureSecret {
    pub value: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum AzureKeyVaultError {
    #[error("Azure Key Vault request failed")]
    Transport(#[source] io::Error),
}

pub trait AzureKeyVaultClient: Send + Sync {
    fn get_secret(&self, secret_name: &str) -> Result<AzureSecret, AzureKeyVaultError>;
}

#[derive(Debug)]
pub struct GoogleKmsDecryptRequest<'request> {
    pub name: &'request str,
    pub ciphertext: &'request [u8],
}

pub trait GoogleKmsClient: Send + Sync {
    fn resource_name(&self) -> &str;

    fn decrypt(&self, request: GoogleKmsDecryptRequest<'_>) -> Result<Vec<u8>, GoogleKmsError>;
}

pub trait AwsKmsClient: Send + Sync {
    fn decrypt(&self, ciphertext_blob: &[u8]) -> Result<Vec<u8>, AwsKmsError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AwsSecretsManagerError {
    #[error("AWS Secrets Manager request failed")]
    Transport(#[source] io::Error),
}

pub trait AwsSecretsManagerClient: Send + Sync {
    fn sync_read_secret(
        &self,
        secret_name: &str,
        primary_secret_name: Option<&str>,
    ) -> Result<Option<SecretString>, AwsSecretsManagerError>;
}

pub trait GoogleSecretManagerClient: Send + Sync {
    // ast-grep-ignore: no-long-names
    fn get_secret_from_google_secret_manager(
        &self,
        secret_name: &str,
    ) -> Result<Option<SecretString>, GoogleSecretManagerError>;
}

// Vault and CyberArk both swallow every read failure into None in LiteLLM
pub trait SyncSecretReader: Send + Sync {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString>;
}

#[derive(Debug)]
pub struct InfisicalSecret {
    pub secret_value: Option<SecretString>,
}

#[derive(Debug, thiserror::Error)]
pub enum InfisicalError {
    #[error("Infisical request failed")]
    Transport(#[source] io::Error),
}

pub trait InfisicalClient: Send + Sync {
    fn get_secret(&self, secret_name: &str) -> Result<InfisicalSecret, InfisicalError>;
}
