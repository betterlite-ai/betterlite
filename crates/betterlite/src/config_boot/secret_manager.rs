use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use betterlite_config::sections::key_management_settings::Settings;
use betterlite_secret_managers::aws::client_init::validate_environment as validate_aws_secret_manager;
use betterlite_secret_managers::aws_kms::initialization::validate_environment as validate_aws_kms;
use betterlite_secret_managers::cyberark::{Clock, CyberArkSecretManager};
use betterlite_secret_managers::google_kms::initialization::validate_environment as validate_google_kms;
use betterlite_secret_managers::google_secret_manager::settings::{
    Options as GoogleOptions, Settings as GoogleSecretSettings,
};
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::conjur_http::ConjurHttpTransport;
use betterlite_secret_managers::secret_read_dispatch::http_transport::TlsVerification;
use betterlite_secret_managers::secret_read_dispatch::reqwest_exchange::ReqwestExchange;
use betterlite_secret_managers::secret_read_dispatch::vault_http::VaultHttpTransport;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use betterlite_secret_managers::vault::HashicorpSecretManager;
use tokio::time::Instant as TokioInstant;

use self::dispatch_params::dispatch_settings;
use super::error::ConfigBootError;

const AZURE_URI_NOT_SET: &str =
    "Error when loading keys from Azure Key Vault: AZURE_KEY_VAULT_URI is not set.";

pub fn select_secret_manager(
    key_management_system: Option<&str>,
    key_management_settings: &Settings,
    config_file_path: Option<&Path>,
    env: &BTreeMap<String, String>,
) -> Result<ManagerState, ConfigBootError> {
    let Some(name) = key_management_system else {
        return Ok(unselected_manager());
    };
    let system = parse_system(name).ok_or(ConfigBootError::InvalidKeyManagementSystem)?;
    let Some(client) = build_client(system, key_management_settings, config_file_path, env)? else {
        return Ok(unselected_manager());
    };
    Ok(ManagerState {
        client,
        settings: Some(dispatch_settings(key_management_settings)),
        system: Some(system),
        access_mode: key_management_settings.access_mode,
        hosted_keys: key_management_settings.hosted_keys.clone(),
    })
}

// LiteLLM compares against KeyManagementSystem's snake_case values; "local" has no branch
fn parse_system(name: &str) -> Option<KeyManagementSystem> {
    match name {
        "azure_key_vault" => Some(KeyManagementSystem::AzureKeyVault),
        "google_kms" => Some(KeyManagementSystem::GoogleKms),
        "aws_secret_manager" => Some(KeyManagementSystem::AwsSecretManager),
        "aws_kms" => Some(KeyManagementSystem::AwsKms),
        "google_secret_manager" => Some(KeyManagementSystem::GoogleSecretManager),
        "hashicorp_vault" => Some(KeyManagementSystem::HashicorpVault),
        "cyberark" => Some(KeyManagementSystem::Cyberark),
        "custom" => Some(KeyManagementSystem::Custom),
        _ => None,
    }
}

const fn unselected_manager() -> ManagerState {
    ManagerState {
        client: SecretManagerClient::Uninitialized,
        settings: None,
        system: None,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

fn build_client(
    system: KeyManagementSystem,
    settings: &Settings,
    config_file_path: Option<&Path>,
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    match system {
        KeyManagementSystem::HashicorpVault => reader_client(ReaderManager::Vault, env),
        KeyManagementSystem::Cyberark => reader_client(ReaderManager::Cyberark, env),
        KeyManagementSystem::AzureKeyVault => Ok(azure_client(env)),
        KeyManagementSystem::GoogleKms => validated_client(
            validate_google_kms(env),
            "initializing the Google KMS client",
        ),
        KeyManagementSystem::AwsKms => {
            validated_client(validate_aws_kms(env), "initializing the AWS KMS client")
        }
        KeyManagementSystem::AwsSecretManager => Ok(Some(aws_secret_client(env))),
        KeyManagementSystem::GoogleSecretManager => validated_client(
            GoogleSecretSettings::resolve(env, GoogleOptions::default()),
            "initializing the Google Secret Manager client",
        ),
        KeyManagementSystem::Custom => custom_client(settings, config_file_path),
        KeyManagementSystem::Local => Err(ConfigBootError::InvalidKeyManagementSystem),
    }
}

// The systems without a betterlite client yet validate their inputs and install NoneType
fn validated_client<T, E>(
    validated: Result<T, E>,
    step: &'static str,
) -> Result<Option<SecretManagerClient>, ConfigBootError>
where
    E: Error + Send + Sync + 'static,
{
    validated.map_err(|source| ConfigBootError::Step {
        step,
        source: Box::new(source),
    })?;
    Ok(Some(SecretManagerClient::Uninitialized))
}

#[derive(Clone, Copy)]
enum ReaderManager {
    Vault,
    Cyberark,
}

// Both readers take the same clock and error step, so one function builds either
fn reader_client(
    reader: ReaderManager,
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    let clock: Clock = Arc::new(TokioInstant::now);
    let exchange = Arc::new(ReqwestExchange::default());
    match reader {
        ReaderManager::Vault => {
            let transport = VaultHttpTransport {
                exchange,
                tls: vault_tls(env),
            };
            let manager = HashicorpSecretManager::new(env, transport, clock)
                .map_err(|source| step_error("initializing the Vault secret manager", source))?;
            Ok(Some(SecretManagerClient::Vault(Arc::new(manager))))
        }
        ReaderManager::Cyberark => {
            let transport = ConjurHttpTransport { exchange };
            let manager = CyberArkSecretManager::new(env, transport, clock)
                .map_err(|source| step_error("initializing the CyberArk secret manager", source))?;
            Ok(Some(SecretManagerClient::CyberArk(Arc::new(manager))))
        }
    }
}

fn step_error<E>(step: &'static str, source: E) -> ConfigBootError
where
    E: Error + Send + Sync + 'static,
{
    ConfigBootError::Step {
        step,
        source: Box::new(source),
    }
}

// LiteLLM gives Vault the httpx client built from get_ssl_verify, which reads SSL_VERIFY
fn vault_tls(env: &BTreeMap<String, String>) -> TlsVerification {
    let configured = env.get("SSL_VERIFY").map_or("", String::as_str);
    if !configured.trim().is_empty() {
        match text_to_bool(configured) {
            Some(true) => {}
            Some(false) => return TlsVerification::Disabled,
            None => return TlsVerification::CaFile(PathBuf::from(configured)),
        }
    }
    if let Some(cert_file) = env.get("SSL_CERT_FILE")
        && Path::new(cert_file).exists()
    {
        return TlsVerification::CaFile(PathBuf::from(cert_file));
    }
    TlsVerification::Enabled
}

// Without a URI LiteLLM installs no client, so secret reads stay on the environment
fn azure_client(env: &BTreeMap<String, String>) -> Option<SecretManagerClient> {
    if env
        .get("AZURE_KEY_VAULT_URI")
        .is_some_and(|uri| !uri.is_empty())
    {
        return Some(SecretManagerClient::Uninitialized);
    }
    tracing::error!(
        "Error when loading keys from Azure Key Vault: {AZURE_URI_NOT_SET} .Ensure you run `pip install azure-identity azure-keyvault-secrets`"
    );
    None
}

fn aws_secret_client(env: &BTreeMap<String, String>) -> SecretManagerClient {
    validate_aws_secret_manager(env);
    SecretManagerClient::Uninitialized
}

const fn custom_client(
    settings: &Settings,
    config_file_path: Option<&Path>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    if config_file_path.is_none() {
        return Err(ConfigBootError::CustomManagerConfigPath);
    }
    if settings.custom_secret_manager.is_none() {
        return Err(ConfigBootError::CustomManagerField);
    }
    Ok(Some(SecretManagerClient::Uninitialized))
}

mod dispatch_params;

mod tests;
