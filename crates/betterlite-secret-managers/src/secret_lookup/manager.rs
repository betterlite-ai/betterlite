use std::collections::BTreeMap;

use secrecy::SecretString;

use super::SecretValue;
use super::values::literal_value;
use crate::secret_read_dispatch::SecretManagerClient;
use crate::secret_read_dispatch::get_secret_from_manager;
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::{KeyManagementSystem, SecretAccessMode};

pub struct ManagerState {
    pub client: SecretManagerClient,
    pub settings: Option<SecretManagerSettings>,
    pub system: Option<KeyManagementSystem>,
    pub access_mode: SecretAccessMode,
    pub hosted_keys: Option<Vec<String>>,
}

pub(super) fn read(
    manager: &ManagerState,
    environment: &BTreeMap<String, String>,
    secret_name: &str,
) -> Option<SecretValue> {
    let key_manager = match &manager.hosted_keys {
        Some(keys) if !keys.iter().any(|key| key.as_str() == secret_name) => {
            KeyManagementSystem::Local
        }
        _ => manager.system.unwrap_or(KeyManagementSystem::Local),
    };
    let read = get_secret_from_manager(
        &manager.client,
        Some(key_manager),
        secret_name,
        manager.settings.as_ref(),
        environment,
    );
    match read {
        Ok(Some(text)) => Some(literal_value(text)),
        Ok(None) => None,
        Err(error) => {
            tracing::error!(
                "Defaulting to os.environ value for key={secret_name}. An exception occurred - {error}."
            );
            environment
                .get(secret_name)
                .map(|text| literal_value(SecretString::from(text.as_str())))
        }
    }
}
