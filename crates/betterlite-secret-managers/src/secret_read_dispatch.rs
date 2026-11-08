use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::SecretString;

use crate::custom_interface::CustomSecretManager;
use crate::secret_types::KeyManagementSystem;
use error::SecretDispatchError;
use protocols::{
    AwsKmsClient, AwsSecretsManagerClient, AzureKeyVaultClient, GoogleKmsClient,
    GoogleSecretManagerClient, InfisicalClient, SyncSecretReader,
};
use settings::SecretManagerSettings;

mod aws_kms;
mod aws_secrets;
mod azure;
pub mod base64_codec;
pub mod conjur_http;
mod custom;
mod cyberark;
pub mod error;
mod google_kms;
mod google_secrets;
pub mod http_transport;
mod infisical;
mod local;
pub mod manager_readers;
pub mod protocols;
pub mod settings;
mod vault;
pub mod vault_http;

mod tests;

pub enum SecretManagerClient {
    AzureKeyVault(Arc<dyn AzureKeyVaultClient>),
    GoogleKms(Arc<dyn GoogleKmsClient>),
    AwsKms(Arc<dyn AwsKmsClient>),
    AwsSecretManager(Arc<dyn AwsSecretsManagerClient>),
    GoogleSecretManager(Arc<dyn GoogleSecretManagerClient>),
    Vault(Arc<dyn SyncSecretReader>),
    CyberArk(Arc<dyn SyncSecretReader>),
    // B4419's trait already carries a sync read with the optional params, so it is the protocol
    Custom(Arc<dyn CustomSecretManager>),
    Infisical(Arc<dyn InfisicalClient>),
    Uninitialized,
}

impl SecretManagerClient {
    #[must_use]
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::AzureKeyVault(_) => "SecretClient",
            Self::GoogleKms(_) => "KeyManagementServiceClient",
            Self::AwsKms(_) => "AWSKeyManagementService",
            Self::AwsSecretManager(_) => "AWSSecretsManagerV2",
            Self::GoogleSecretManager(_) => "GoogleSecretManager",
            Self::Vault(_) => "HashicorpSecretManager",
            Self::CyberArk(_) => "CyberArkSecretManager",
            Self::Custom(_) => "CustomSecretManager",
            Self::Infisical(_) => "InfisicalSecretManager",
            Self::Uninitialized => "NoneType",
        }
    }
}

pub fn get_secret_from_manager(
    client: &SecretManagerClient,
    key_manager: Option<KeyManagementSystem>,
    secret_name: &str,
    settings: Option<&SecretManagerSettings>,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, SecretDispatchError> {
    // LiteLLM matches the Azure client's own class name as well as the configured system
    if key_manager == Some(KeyManagementSystem::AzureKeyVault)
        || matches!(client, SecretManagerClient::AzureKeyVault(_))
    {
        return azure::read(client, secret_name);
    }
    // LiteLLM matches the Google KMS client's own class name as well as the configured system
    if key_manager == Some(KeyManagementSystem::GoogleKms)
        || matches!(client, SecretManagerClient::GoogleKms(_))
    {
        return google_kms::read(client, secret_name, environment);
    }
    match key_manager {
        // the two duck-typed managers above already returned, so only exhaustiveness reaches here
        Some(KeyManagementSystem::AzureKeyVault) => azure::read(client, secret_name),
        Some(KeyManagementSystem::GoogleKms) => google_kms::read(client, secret_name, environment),
        Some(KeyManagementSystem::AwsKms) => aws_kms::read(client, secret_name, environment),
        Some(KeyManagementSystem::AwsSecretManager) => {
            aws_secrets::read(client, secret_name, settings)
        }
        Some(KeyManagementSystem::GoogleSecretManager) => google_secrets::read(client, secret_name),
        Some(KeyManagementSystem::HashicorpVault) => vault::read(client, secret_name),
        Some(KeyManagementSystem::Cyberark) => cyberark::read(client, secret_name),
        Some(KeyManagementSystem::Custom) => custom::read(client, secret_name, settings),
        Some(KeyManagementSystem::Local) => Ok(local::read(environment, secret_name)),
        // LiteLLM's else branch treats a key manager it does not know as the Infisical default
        None => infisical::read(client, secret_name),
    }
}
