use betterlite_config::config_override::CYBERARK_FIELDS;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secret_managers::secret_types::KeyManagementSystem;

use super::environment::{EnvironmentSnapshot, RuntimeEnvironment, project_environment};
use super::models::OverrideKind;

#[derive(Debug, thiserror::Error)]
#[error("the secret manager failed to initialize: {message}")]
pub struct SecretManagerInitError {
    pub message: String,
}

// The proxy's one active secret manager and the two change-detection markers the routes own.
pub trait SecretManagerControl {
    fn active_manager(&self) -> Option<KeyManagementSystem>;

    fn drop_active_manager(&mut self);

    fn initialize(&mut self, kind: KeyManagementSystem) -> Result<(), SecretManagerInitError>;
}

#[derive(Debug, Default)]
pub struct OverrideRuntimeState {
    pub vault_last_config: Option<JsonValue>,
    pub cyberark_last_config: Option<JsonValue>,
    pub cyberark_boot_env: Option<JsonMap>,
}

pub fn clear_hashicorp_vault_state(
    environment: &mut RuntimeEnvironment,
    state: &mut OverrideRuntimeState,
    control: &mut dyn SecretManagerControl,
) {
    environment.apply(&project_environment(
        OverrideKind::HashicorpVault,
        &JsonMap::new(),
    ));
    if control.active_manager() == Some(KeyManagementSystem::HashicorpVault) {
        control.drop_active_manager();
    }
    state.vault_last_config = None;
}

pub fn snapshot_cyberark_boot_env(
    state: &mut OverrideRuntimeState,
    snapshot: &EnvironmentSnapshot,
) {
    if state.cyberark_boot_env.is_some() {
        return;
    }
    let mut boot = JsonMap::new();
    for field in CYBERARK_FIELDS {
        let value = snapshot
            .values
            .get(field.env_var)
            .map_or(JsonValue::Null, |text| JsonValue::String(text.clone()));
        boot.insert(field.name.to_owned(), value);
    }
    state.cyberark_boot_env = Some(boot);
}

// _restore_cyberark_runtime: apply the values, try CyberArk, and only an active CyberArk may hand back to Vault.
pub fn restore_cyberark_runtime(
    environment: &mut RuntimeEnvironment,
    state: &mut OverrideRuntimeState,
    control: &mut dyn SecretManagerControl,
    values: &JsonMap,
) {
    let batch = project_environment(OverrideKind::CyberArk, values);
    let has_api_base = batch
        .changes
        .get("CYBERARK_API_BASE")
        .is_some_and(Option::is_some);
    environment.apply(&batch);
    if has_api_base {
        match control.initialize(KeyManagementSystem::Cyberark) {
            Ok(()) => return,
            Err(error) => {
                tracing::error!("Failed to restore previous CyberArk configuration: {error}");
            }
        }
    }
    if control.active_manager() != Some(KeyManagementSystem::Cyberark) {
        return;
    }
    control.drop_active_manager();
    state.vault_last_config = None;
    let vault_addr = environment
        .snapshot()
        .values
        .get("HCP_VAULT_ADDR")
        .cloned()
        .unwrap_or_default();
    if vault_addr.is_empty() {
        return;
    }
    if let Err(error) = control.initialize(KeyManagementSystem::HashicorpVault) {
        tracing::error!("Failed to reinitialize Hashicorp Vault after CyberArk rollback: {error}");
    }
}

pub fn clear_cyberark_state(
    environment: &mut RuntimeEnvironment,
    state: &mut OverrideRuntimeState,
    control: &mut dyn SecretManagerControl,
) {
    let boot = state.cyberark_boot_env.clone().unwrap_or_default();
    restore_cyberark_runtime(environment, state, control, &boot);
    state.cyberark_last_config = None;
}
