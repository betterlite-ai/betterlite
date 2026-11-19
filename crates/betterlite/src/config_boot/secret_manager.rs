use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use betterlite_config::sections::key_management_settings::Settings;
use betterlite_secret_managers::aws::client_init::validate_environment as validate_aws_secret_manager;
use betterlite_secret_managers::aws_kms::initialization::validate_environment as validate_aws_kms;
use betterlite_secret_managers::custom_interface::{CustomParams, CustomValue};
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
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_read_dispatch::vault_http::VaultHttpTransport;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use betterlite_secret_managers::vault::HashicorpSecretManager;
use secrecy::ExposeSecret as _;
use tokio::time::Instant as TokioInstant;

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
        KeyManagementSystem::HashicorpVault => vault_client(env),
        KeyManagementSystem::Cyberark => cyberark_client(env),
        KeyManagementSystem::AzureKeyVault => Ok(azure_client(env)),
        KeyManagementSystem::GoogleKms => google_kms_client(env),
        KeyManagementSystem::AwsKms => aws_kms_client(env),
        KeyManagementSystem::AwsSecretManager => Ok(Some(aws_secret_client(env))),
        KeyManagementSystem::GoogleSecretManager => google_secret_client(env),
        KeyManagementSystem::Custom => custom_client(settings, config_file_path),
        KeyManagementSystem::Local => Err(ConfigBootError::InvalidKeyManagementSystem),
    }
}

fn vault_client(
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    let transport = VaultHttpTransport {
        exchange: Arc::new(ReqwestExchange::default()),
        tls: vault_tls(env),
    };
    let clock: Clock = Arc::new(TokioInstant::now);
    let manager = HashicorpSecretManager::new(env, transport, clock).map_err(|source| {
        ConfigBootError::Step {
            step: "initializing the Vault secret manager",
            source: Box::new(source),
        }
    })?;
    Ok(Some(SecretManagerClient::Vault(Arc::new(manager))))
}

fn cyberark_client(
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    let transport = ConjurHttpTransport {
        exchange: Arc::new(ReqwestExchange::default()),
    };
    let clock: Clock = Arc::new(TokioInstant::now);
    let manager = CyberArkSecretManager::new(env, transport, clock).map_err(|source| {
        ConfigBootError::Step {
            step: "initializing the CyberArk secret manager",
            source: Box::new(source),
        }
    })?;
    Ok(Some(SecretManagerClient::CyberArk(Arc::new(manager))))
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

fn google_kms_client(
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    let _validated = validate_google_kms(env).map_err(|source| ConfigBootError::Step {
        step: "initializing the Google KMS client",
        source: Box::new(source),
    })?;
    Ok(Some(SecretManagerClient::Uninitialized))
}

fn aws_kms_client(
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    let _validated = validate_aws_kms(env).map_err(|source| ConfigBootError::Step {
        step: "initializing the AWS KMS client",
        source: Box::new(source),
    })?;
    Ok(Some(SecretManagerClient::Uninitialized))
}

fn aws_secret_client(env: &BTreeMap<String, String>) -> SecretManagerClient {
    validate_aws_secret_manager(env);
    SecretManagerClient::Uninitialized
}

fn google_secret_client(
    env: &BTreeMap<String, String>,
) -> Result<Option<SecretManagerClient>, ConfigBootError> {
    GoogleSecretSettings::resolve(env, GoogleOptions::default()).map_err(|source| {
        ConfigBootError::Step {
            step: "initializing the Google Secret Manager client",
            source: Box::new(source),
        }
    })?;
    Ok(Some(SecretManagerClient::Uninitialized))
}

fn custom_client(
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

fn dispatch_settings(settings: &Settings) -> SecretManagerSettings {
    SecretManagerSettings {
        primary_secret_name: settings.primary_secret_name.clone(),
        optional_params: custom_optional_params(settings),
    }
}

fn custom_optional_params(settings: &Settings) -> CustomParams {
    let mut params = CustomParams::new();
    custom_store_params(&mut params, settings);
    custom_text_params(&mut params, settings);
    custom_aws_params(&mut params, settings);
    params
}

fn custom_store_params(params: &mut CustomParams, settings: &Settings) {
    let hosted = settings
        .hosted_keys
        .as_ref()
        .map_or(CustomValue::Null, |names| {
            CustomValue::Array(names.iter().cloned().map(CustomValue::String).collect())
        });
    params.insert("hosted_keys".to_owned(), hosted);
    params.insert(
        "store_virtual_keys".to_owned(),
        CustomValue::Bool(settings.store_virtual_keys),
    );
    params.insert(
        "prefix_for_stored_virtual_keys".to_owned(),
        CustomValue::String(settings.prefix_for_stored_virtual_keys.clone()),
    );
    params.insert(
        "access_mode".to_owned(),
        CustomValue::String(access_mode_name(settings.access_mode).to_owned()),
    );
}

fn custom_text_params(params: &mut CustomParams, settings: &Settings) {
    params.insert(
        "primary_secret_name".to_owned(),
        settings
            .primary_secret_name
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "description".to_owned(),
        settings
            .description
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    let tags = settings.tags.as_ref().map_or(CustomValue::Null, |tags| {
        CustomValue::Object(
            tags.iter()
                .map(|(name, value)| (name.clone(), CustomValue::String(value.clone())))
                .collect(),
        )
    });
    params.insert("tags".to_owned(), tags);
    params.insert(
        "kms_key_id".to_owned(),
        settings
            .kms_key_id
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "custom_secret_manager".to_owned(),
        settings
            .custom_secret_manager
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
}

fn custom_aws_params(params: &mut CustomParams, settings: &Settings) {
    params.insert(
        "aws_region_name".to_owned(),
        settings
            .aws_region_name
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "aws_role_name".to_owned(),
        settings
            .aws_role_name
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "aws_session_name".to_owned(),
        settings
            .aws_session_name
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "aws_external_id".to_owned(),
        settings
            .aws_external_id
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    custom_aws_identity_params(params, settings);
}

fn custom_aws_identity_params(params: &mut CustomParams, settings: &Settings) {
    params.insert(
        "aws_profile_name".to_owned(),
        settings
            .aws_profile_name
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "aws_web_identity_token".to_owned(),
        settings
            .aws_web_identity_token
            .as_ref()
            .map_or(CustomValue::Null, |token| {
                CustomValue::String(token.expose_secret().to_owned())
            }),
    );
    params.insert(
        "aws_sts_endpoint".to_owned(),
        settings
            .aws_sts_endpoint
            .clone()
            .map_or(CustomValue::Null, CustomValue::String),
    );
    params.insert(
        "replica_regions".to_owned(),
        CustomValue::Array(
            settings
                .replica_regions
                .iter()
                .cloned()
                .map(CustomValue::String)
                .collect(),
        ),
    );
}

const fn access_mode_name(mode: SecretAccessMode) -> &'static str {
    match mode {
        SecretAccessMode::ReadOnly => "read_only",
        SecretAccessMode::WriteOnly => "write_only",
        SecretAccessMode::ReadAndWrite => "read_and_write",
    }
}

mod tests;
