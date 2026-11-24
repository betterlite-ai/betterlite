use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use betterlite_config::sections::settings::{FieldSource, JsonValue, Section, SettingsStore};

use super::error::{ConfigOwnedDetail, ConfigStateError};

const STORE_MODEL_IN_DB: &str = "store_model_in_db";

// LiteLLM's ProxyConfig between requests: the four settings stores, the snapshot and the file an admin edit is refused with
pub struct ProxyConfigState {
    pub general_settings: SettingsStore,
    pub router_settings: SettingsStore,
    pub litellm_settings: SettingsStore,
    pub environment_variables: SettingsStore,
    pub config_file_path: PathBuf,
    pub store_model_in_db: bool,
    pub(crate) snapshot: BTreeMap<String, JsonValue>,
}

impl ProxyConfigState {
    #[must_use]
    pub fn new(config_file_path: PathBuf, store_model_in_db: bool) -> Self {
        Self {
            general_settings: SettingsStore::from(Section::GeneralSettings),
            router_settings: SettingsStore::from(Section::RouterSettings),
            litellm_settings: SettingsStore::from(Section::LitellmSettings),
            environment_variables: SettingsStore::from(Section::EnvironmentVariables),
            config_file_path,
            store_model_in_db,
            snapshot: BTreeMap::new(),
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions - LiteLLM's get_config_state is this named deep copy, and B8379 reads the config through it
    pub fn snapshot(&self) -> BTreeMap<String, JsonValue> {
        self.snapshot.clone()
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's update_config_state is this named deep copy, and every save ends on it
    pub fn replace_snapshot(&mut self, config: &BTreeMap<String, JsonValue>) {
        self.snapshot = config.clone();
    }

    #[must_use]
    pub const fn settings_store(&self, section: Section) -> Option<&SettingsStore> {
        match section {
            Section::GeneralSettings => Some(&self.general_settings),
            Section::RouterSettings => Some(&self.router_settings),
            Section::LitellmSettings => Some(&self.litellm_settings),
            Section::EnvironmentVariables => Some(&self.environment_variables),
            Section::UiSettings => None,
        }
    }

    pub(crate) const fn settings_store_mut(
        &mut self,
        section: Section,
    ) -> Option<&mut SettingsStore> {
        match section {
            Section::GeneralSettings => Some(&mut self.general_settings),
            Section::RouterSettings => Some(&mut self.router_settings),
            Section::LitellmSettings => Some(&mut self.litellm_settings),
            Section::EnvironmentVariables => Some(&mut self.environment_variables),
            Section::UiSettings => None,
        }
    }

    // LiteLLM's `general_settings.get("store_model_in_db", False) is True or store_model_in_db`
    #[must_use]
    pub fn store_model_in_db_enabled(&self) -> bool {
        self.store_model_in_db
            || self.general_settings.get(STORE_MODEL_IN_DB) == Some(&JsonValue::Bool(true))
    }

    // A write the config file owns is refused unless it resends the file's value, which never reaches the store
    pub fn reject_config_owned_writes(
        &self,
        section: Section,
        changed: &BTreeMap<String, JsonValue>,
    ) -> Result<(), ConfigStateError> {
        let Some(store) = self.settings_store(section) else {
            return Ok(());
        };
        let rejected = store.rejected_writes(changed);
        if rejected.is_empty() {
            return Ok(());
        }
        Err(self.config_owned_error(section, &rejected, store))
    }

    // A delete has no value to compare, so any key the file owns is refused
    pub fn reject_config_owned_deletes(
        &self,
        section: Section,
        keys: &[String],
    ) -> Result<(), ConfigStateError> {
        let Some(store) = self.settings_store(section) else {
            return Ok(());
        };
        let owned: BTreeSet<String> = keys
            .iter()
            .filter(|key| store.resolution(key.as_str()).source() == FieldSource::Config)
            .cloned()
            .collect();
        if owned.is_empty() {
            return Ok(());
        }
        let rejected: Vec<String> = owned.into_iter().collect();
        Err(self.config_owned_error(section, &rejected, store))
    }

    fn config_owned_error(
        &self,
        section: Section,
        rejected: &[String],
        store: &SettingsStore,
    ) -> ConfigStateError {
        let subject = match rejected {
            [key] => format!("key '{key}' is"),
            keys => {
                let quoted: Vec<String> = keys.iter().map(|key| format!("'{key}'")).collect();
                format!("keys {} are", quoted.join(", "))
            }
        };
        let pronoun = if rejected.len() == 1 { "it" } else { "them" };
        let shadowed: Vec<String> = rejected
            .iter()
            .filter(|key| store.shadows_db_value(key.as_str()))
            .cloned()
            .collect();
        let stored = stored_values_sentence(&shadowed);
        let resolution = format!(
            "edit {} to change {pronoun}, or remove {pronoun} from the file to let the database own {pronoun}",
            self.config_file_path.display()
        );
        ConfigStateError::ConfigOwned(Box::new(ConfigOwnedDetail {
            error: format!(
                "{} {subject} set in the config file and cannot be changed here.{stored}",
                section.name()
            ),
            keys: rejected.to_vec(),
            section: section.name().to_owned(),
            stored_database_values_ignored: shadowed,
            resolution,
        }))
    }
}

fn stored_values_sentence(shadowed: &[String]) -> String {
    if shadowed.is_empty() {
        return String::new();
    }
    let value = if shadowed.len() == 1 {
        "value"
    } else {
        "values"
    };
    let is = if shadowed.len() == 1 { "is" } else { "are" };
    format!(
        " The {value} already stored in the database for {} {is} ignored and will never be applied.",
        shadowed.join(", ")
    )
}
