mod tests;

use std::collections::BTreeMap;
use std::path::Path;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::key_management_settings::{self, Settings as KeySettings};
use betterlite_config::sections::secret_manager_bootstrap::Settings as BootstrapSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::tls::VerifySetting;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_types::KeyManagementSystem;
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::error::ConfigBootError;
use super::secret_manager::select_secret_manager;

const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Error)]
pub enum RawSecretManagerError {
    #[error("could not decode the key management settings mapping")]
    SettingsValue {
        #[from]
        source: serde_json::Error,
    },
    #[error("could not resolve the key management settings")]
    Settings {
        #[from]
        source: ConfigError,
    },
    #[error("could not resolve the key management system reference")]
    System {
        #[from]
        source: SecretLookupError,
    },
    #[error("could not select the secret manager")]
    Selection {
        #[from]
        source: ConfigBootError,
    },
}

pub struct RawSecretManager {
    pub lookup: SecretLookup,
    pub settings: Option<KeySettings>,
}

impl RawSecretManager {
    #[must_use]
    pub const fn new(lookup: SecretLookup) -> Self {
        Self {
            lookup,
            settings: None,
        }
    }

    pub fn initialize(
        &mut self,
        projection: &BootstrapSettings,
        config_file_path: Option<&Path>,
        env: &BTreeMap<String, String>,
        litellm_ssl_verify: Option<&VerifySetting>,
    ) -> Result<(), RawSecretManagerError> {
        if self.has_client() {
            return Ok(());
        }
        let Some(raw_system) = projection.key_management_system.as_deref() else {
            return Ok(());
        };
        let Some(system) = self.resolve_system(raw_system, env)? else {
            return Ok(());
        };
        if let Some(settings) = projection
            .key_management_settings
            .as_ref()
            .filter(|value| value.is_object())
        {
            self.settings = Some(resolved_settings(settings, &system, env)?);
        }
        let settings = self.settings.clone().unwrap_or_default();
        let state = select_secret_manager(
            Some(system.as_str()),
            &settings,
            config_file_path,
            env,
            litellm_ssl_verify,
        )?;
        self.lookup.manager = Some(state);
        Ok(())
    }

    // LiteLLM returns on secret_manager_client is not None; the NoneType placeholder stands for no client
    fn has_client(&self) -> bool {
        self.lookup
            .manager
            .as_ref()
            .is_some_and(|state| !matches!(state.client, SecretManagerClient::Uninitialized))
    }

    fn resolve_system(
        &self,
        raw_system: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Option<String>, RawSecretManagerError> {
        let Some(variable) = raw_system.strip_prefix(ENVIRONMENT_PREFIX) else {
            return Ok(Some(raw_system.to_owned()));
        };
        let resolved = self.lookup.get_secret_str(env, variable, None)?;
        Ok(resolved.map(|secret| secret.expose_secret().to_owned()))
    }
}

// LiteLLM resolves the manager settings on a deep copy, so the config keeps its references for the main pass
fn resolved_settings(
    value: &JsonValue,
    system: &str,
    env: &BTreeMap<String, String>,
) -> Result<KeySettings, RawSecretManagerError> {
    let raw_settings =
        serde_json::from_value::<key_management_settings::RawSettings>(value.clone())?;
    let known_system =
        serde_json::from_value::<KeyManagementSystem>(JsonValue::String(system.to_owned())).ok();
    let raw = key_management_settings::Raw {
        general_settings: key_management_settings::RawGeneral {
            key_management_settings: Some(raw_settings),
            key_management_system: known_system,
        },
    };
    Ok(raw.convert(env)?)
}
