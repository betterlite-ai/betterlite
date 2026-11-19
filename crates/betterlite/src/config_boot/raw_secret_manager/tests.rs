#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup, parse_view};
use betterlite_config::sections::Section as _;
use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::secret_manager_bootstrap::Raw as BootstrapRaw;
use betterlite_config::sections::secret_manager_bootstrap::Settings as BootstrapSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::custom_interface::CustomParams;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::{ExposeSecret as _, SecretString};

use super::{RawSecretManager, RawSecretManagerError};
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::test_support::{capture_logs, config_from_yaml, secret_lookup};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn project(source: &str, lookup: &BTreeMap<String, String>) -> BootstrapSettings {
    let (raw, _ignored) = parse_view::<BootstrapRaw>(source).unwrap();
    raw.convert(lookup).unwrap()
}

fn project_map(config: &ConfigMap) -> BootstrapSettings {
    let value = JsonValue::Object(config.clone().into_iter().collect());
    let raw = serde_json::from_value::<BootstrapRaw>(value).unwrap();
    raw.convert(&BTreeMap::<String, String>::new()).unwrap()
}

struct RecordingReader {
    reads: Mutex<Vec<String>>,
    value: &'static str,
}

impl SyncSecretReader for RecordingReader {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(secret_name.to_owned());
        Some(SecretString::from(self.value))
    }
}

fn recording_manager(reader: &Arc<RecordingReader>) -> ManagerState {
    ManagerState {
        client: SecretManagerClient::Vault(Arc::clone(reader)),
        settings: Some(SecretManagerSettings {
            primary_secret_name: None,
            optional_params: CustomParams::new(),
        }),
        system: Some(KeyManagementSystem::HashicorpVault),
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

struct StagedLookup<'state> {
    secrets: &'state SecretLookup,
    environment: &'state BTreeMap<String, String>,
}

impl EnvironmentLookup for StagedLookup<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.secrets
            .get_secret_str(self.environment, name, None)
            .ok()
            .flatten()
            .map(|secret| secret.expose_secret().to_owned())
    }
}

#[test]
fn absent_system_noop() {
    let mut state = RawSecretManager::new(secret_lookup(None));
    let projection = project(
        "general_settings:\n  key_management_settings:\n    hosted_keys: [token]\n",
        &env(&[]),
    );
    state
        .initialize(&projection, None, &env(&[]), None)
        .unwrap();
    assert!(state.lookup.manager.is_none());
    assert!(state.settings.is_none());
    let numeric = project("general_settings:\n  key_management_system: 5\n", &env(&[]));
    state.initialize(&numeric, None, &env(&[]), None).unwrap();
    assert!(state.lookup.manager.is_none());
}

#[test]
fn system_from_environment() {
    let environment = env(&[("KMS", "hashicorp_vault"), ("HCP_VAULT_TOKEN", "token")]);
    let projection = project(
        "general_settings:\n  key_management_system: os.environ/KMS\n",
        &environment,
    );
    assert_eq!(
        projection.key_management_system.as_deref(),
        Some("os.environ/KMS")
    );
    let mut state = RawSecretManager::new(secret_lookup(None));
    state
        .initialize(&projection, None, &environment, None)
        .unwrap();
    let manager = state.lookup.manager.as_ref().unwrap();
    assert_eq!(manager.system, Some(KeyManagementSystem::HashicorpVault));
    assert_eq!(manager.client.class_name(), "HashicorpSecretManager");
    let missing = project(
        "general_settings:\n  key_management_system: os.environ/ABSENT\n",
        &environment,
    );
    let mut absent = RawSecretManager::new(secret_lookup(None));
    absent
        .initialize(&missing, None, &environment, None)
        .unwrap();
    assert!(absent.lookup.manager.is_none());
}

#[test]
fn existing_client_skips_new_settings() {
    let reader = Arc::new(RecordingReader {
        reads: Mutex::new(Vec::new()),
        value: "sk-manager",
    });
    let mut state = RawSecretManager::new(secret_lookup(Some(recording_manager(&reader))));
    let projection = project(
        "general_settings:\n  key_management_system: aws_secret_manager\n  key_management_settings:\n    access_mode: bogus\n",
        &env(&[]),
    );
    state
        .initialize(
            &projection,
            None,
            &env(&[("AWS_REGION_NAME", "us-east-1")]),
            None,
        )
        .unwrap();
    let manager = state.lookup.manager.as_ref().unwrap();
    assert_eq!(manager.system, Some(KeyManagementSystem::HashicorpVault));
    assert_eq!(manager.client.class_name(), "HashicorpSecretManager");
    assert!(state.settings.is_none());
}

#[test]
fn raw_config_unchanged() {
    let config = config_from_yaml(
        "general_settings:\n  key_management_system: hashicorp_vault\n  key_management_settings:\n    kms_key_id: os.environ/KMS_ID\n    tags:\n      Environment: os.environ/TAG\nmodel_list:\n- model_name: gpt\n  litellm_params:\n    model: openai/gpt-4o-mini\n    api_key: os.environ/MODEL_KEY\n",
    );
    let original = config.clone();
    let projection = project_map(&config);
    let environment = env(&[
        ("HCP_VAULT_TOKEN", "token"),
        ("KMS_ID", "kms-id"),
        ("TAG", "Prod"),
    ]);
    let mut state = RawSecretManager::new(secret_lookup(None));
    state
        .initialize(&projection, None, &environment, None)
        .unwrap();
    assert_eq!(config, original);
    let settings = state.settings.as_ref().unwrap();
    assert_eq!(settings.kms_key_id.as_deref(), Some("kms-id"));
    assert_eq!(
        settings
            .tags
            .as_ref()
            .unwrap()
            .get("Environment")
            .map(String::as_str),
        Some("Prod")
    );
    let raw_settings = projection.key_management_settings.as_ref().unwrap();
    assert_eq!(raw_settings["kms_key_id"], "os.environ/KMS_ID");
    assert_eq!(raw_settings["tags"]["Environment"], "os.environ/TAG");
}

#[test]
fn settings_before_selection() {
    let environment = env(&[("AWS_REGION_NAME", "us-east-1")]);
    let projection = project(
        "general_settings:\n  key_management_system: aws_secret_manager\n  key_management_settings:\n    primary_secret_name: primary\n    access_mode: read_and_write\n",
        &environment,
    );
    let mut state = RawSecretManager::new(secret_lookup(None));
    state
        .initialize(&projection, None, &environment, None)
        .unwrap();
    let manager = state.lookup.manager.as_ref().unwrap();
    assert_eq!(manager.system, Some(KeyManagementSystem::AwsSecretManager));
    assert_eq!(manager.access_mode, SecretAccessMode::ReadAndWrite);
    assert_eq!(
        manager
            .settings
            .as_ref()
            .unwrap()
            .primary_secret_name
            .as_deref(),
        Some("primary")
    );
}

#[test]
fn selection_error_keeps_settings() {
    let projection = project(
        "general_settings:\n  key_management_system: custom\n  key_management_settings:\n    custom_secret_manager: package.Manager\n    access_mode: write_only\n",
        &env(&[]),
    );
    let mut state = RawSecretManager::new(secret_lookup(None));
    let error = state
        .initialize(&projection, None, &env(&[]), None)
        .unwrap_err();
    assert!(matches!(
        error,
        RawSecretManagerError::Selection {
            source: ConfigBootError::CustomManagerConfigPath
        }
    ));
    assert!(state.lookup.manager.is_none());
    let settings = state.settings.as_ref().unwrap();
    assert_eq!(settings.access_mode, SecretAccessMode::WriteOnly);
    assert_eq!(
        settings.custom_secret_manager.as_deref(),
        Some("package.Manager")
    );
}

#[test]
fn missing_azure_uri_can_retry() {
    let (logs, _guard) = capture_logs();
    let projection = project(
        "general_settings:\n  key_management_system: azure_key_vault\n",
        &env(&[]),
    );
    let mut state = RawSecretManager::new(secret_lookup(None));
    state
        .initialize(&projection, None, &env(&[]), None)
        .unwrap();
    let missing = state.lookup.manager.as_ref().unwrap();
    assert!(missing.system.is_none());
    assert_eq!(missing.client.class_name(), "NoneType");
    assert_eq!(
        logs.contents()
            .matches("AZURE_KEY_VAULT_URI is not set")
            .count(),
        1
    );
    let environment = env(&[("AZURE_KEY_VAULT_URI", "https://vault.azure.net")]);
    state
        .initialize(&projection, None, &environment, None)
        .unwrap();
    let manager = state.lookup.manager.as_ref().unwrap();
    assert_eq!(manager.system, Some(KeyManagementSystem::AzureKeyVault));
    assert!(manager.settings.is_some());
    assert_eq!(
        logs.contents()
            .matches("AZURE_KEY_VAULT_URI is not set")
            .count(),
        1
    );
}

#[test]
fn manager_only_credential_resolves_on_first_load() {
    let source = "model_list:\n- model_name: gpt\n  litellm_params:\n    model: openai/gpt-4o-mini\n    api_key: os.environ/MODEL_KEY\ngeneral_settings:\n  key_management_system: hashicorp_vault\n";
    let environment = env(&[("HCP_VAULT_TOKEN", "token")]);
    let projection = project(source, &environment);
    let mut state = RawSecretManager::new(secret_lookup(None));
    state
        .initialize(&projection, None, &environment, None)
        .unwrap();
    assert_eq!(
        state.lookup.manager.as_ref().unwrap().system,
        Some(KeyManagementSystem::HashicorpVault)
    );
    // a Vault read needs its server, so the shared lookup serves the staged load from this reader instead
    let reader = Arc::new(RecordingReader {
        reads: Mutex::new(Vec::new()),
        value: "sk-manager",
    });
    state.lookup.manager = Some(recording_manager(&reader));
    let config = ConfigLoader {
        lookup: StagedLookup {
            secrets: &state.lookup,
            environment: &environment,
        },
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.model_list[0]
            .litellm_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-manager")
    );
    {
        let recorded = reader.reads.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(*recorded, vec![String::from("MODEL_KEY")]);
    }
    state
        .initialize(
            &project(
                "general_settings:\n  key_management_system: aws_secret_manager\n",
                &environment,
            ),
            None,
            &environment,
            None,
        )
        .unwrap();
    assert_eq!(
        state.lookup.manager.as_ref().unwrap().system,
        Some(KeyManagementSystem::HashicorpVault)
    );
    assert_eq!(
        state.lookup.manager.as_ref().unwrap().client.class_name(),
        "HashicorpSecretManager"
    );
}
