use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::settings_resolution::JsonValue as Value;
use thiserror::Error;

use super::descriptors::FieldSource;
use super::settings_resolution::{resolve, Resolved, SettingValue};
use super::settings_rules::{db_row_for, empty_values};

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ConfigOwnedKeyError {
    pub section: String,
    pub key: String,
    pub shadows_db_value: bool,
    pub message: String,
}

#[must_use]
pub fn config_ownership_message(section: &str, key: &str, shadows_db_value: bool) -> String {
    let stored = if shadows_db_value {
        " The value stored in the database for it is ignored and will never be applied."
    } else {
        ""
    };
    format!("{section}.{key} is set in the config file, so the config file owns it and it cannot be changed here.{stored} Edit the config file to change it, or remove it from the file to let the database own it.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    GeneralSettings,
    RouterSettings,
    LiteLlmSettings,
    EnvironmentVariables,
    UiSettings,
}

impl Section {
    const fn as_str(self) -> &'static str {
        match self {
            Self::GeneralSettings => "general_settings",
            Self::RouterSettings => "router_settings",
            Self::LiteLlmSettings => "litellm_settings",
            Self::EnvironmentVariables => "environment_variables",
            Self::UiSettings => "ui_settings",
        }
    }
}

impl From<&str> for Section {
    fn from(value: &str) -> Self {
        match value {
            "router_settings" => Self::RouterSettings,
            "litellm_settings" => Self::LiteLlmSettings,
            "environment_variables" => Self::EnvironmentVariables,
            "ui_settings" => Self::UiSettings,
            _ => Self::GeneralSettings,
        }
    }
}

impl From<String> for Section {
    fn from(value: String) -> Self { Self::from(value.as_str()) }
}

pub struct SettingsStore {
    section: Section,
    yaml_values: BTreeMap<String, Value>,
    database_rows: BTreeMap<Section, BTreeMap<String, Value>>,
    runtime_values: BTreeMap<String, Value>,
    deleted_runtime_keys: BTreeSet<String>,
}

#[cfg(test)]
mod tests;

impl fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsStore").field("section", &self.section.as_str()).finish_non_exhaustive()
    }
}

impl SettingsStore {
    #[must_use]
    pub const fn new(section: Section) -> Self {
        Self { section, yaml_values: BTreeMap::new(), database_rows: BTreeMap::new(), runtime_values: BTreeMap::new(), deleted_runtime_keys: BTreeSet::new() }
    }

    pub fn load_yaml(&mut self, values: BTreeMap<String, Value>) {
        let resource = self.section == Section::GeneralSettings;
        self.yaml_values = values
            .into_iter()
            .filter(|(key, _)| !(resource && key == "pass_through_endpoints"))
            .collect();
        self.runtime_values.retain(|key, _| resource && key == "pass_through_endpoints");
        self.deleted_runtime_keys.clear();
    }

    pub fn apply_db_row(&mut self, row: Section, values: BTreeMap<String, Value>) {
        let previous = self.database_rows.get(&row).cloned().unwrap_or_else(empty_values);
        let changed: BTreeSet<String> = previous.keys().chain(values.keys()).filter(|key| previous.get(*key) != values.get(*key)).cloned().collect();
        self.database_rows.insert(row, values);
        for key in changed {
            if !self.owned_by_config(&key) { self.runtime_values.remove(&key); self.deleted_runtime_keys.remove(&key); }
        }
    }

    pub fn apply_runtime_values(&mut self, values: BTreeMap<String, Value>) {
        self.runtime_values = values
            .into_iter()
            .filter(|(key, _)| !self.owned_by_config(key))
            .collect();
        self.deleted_runtime_keys.clear();
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        if self.deleted_runtime_keys.contains(key) { return None; }
        if let Some(value) = self.runtime_values.get(key) { return Some(value); }
        self.resolved_value(key)
    }

    pub fn set(&mut self, key: &str, value: Value) -> Result<(), ConfigOwnedKeyError> {
        if self.owned_by_config(key) && self.get(key) != Some(&value) { return Err(self.ownership_error(key)); }
        self.runtime_values.insert(key.to_owned(), value);
        self.deleted_runtime_keys.remove(key);
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<(), ConfigOwnedKeyError> {
        if self.get(key).is_none() { return Ok(()); }
        if self.owned_by_config(key) { return Err(self.ownership_error(key)); }
        self.runtime_values.remove(key);
        self.deleted_runtime_keys.insert(key.to_owned());
        Ok(())
    }

    #[must_use]
    pub fn resolved(&self) -> BTreeMap<String, Value> { self.iter().filter_map(|key| self.get(&key).cloned().map(|value| (key, value))).collect() }
    #[must_use]
    pub fn source(&self, key: &str) -> FieldSource { self.resolution_for(key).source }
    #[must_use]
    pub fn config_value(&self, key: &str) -> Option<&Value> { self.yaml_values.get(key) }
    #[must_use]
    pub fn owned_by_config(&self, key: &str) -> bool { self.yaml_values.contains_key(key) }
    #[must_use]
    pub fn db_value(&self, key: &str) -> Option<&Value> { if self.is_resource_list(key) { self.db_value_inner(key) } else { None } }
    pub fn iter(&self) -> impl Iterator<Item = String> + '_ { self.keys().into_iter().filter(|key| !self.deleted_runtime_keys.contains(key) && self.get(key).is_some()) }
    #[must_use]
    pub fn len(&self) -> usize { self.iter().count() }
    #[must_use]
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    #[must_use]
    pub fn rejected_writes(&self, incoming: &BTreeMap<String, Value>) -> Vec<String> { incoming.keys().filter(|key| self.owned_by_config(key) && self.get(key.as_str()) != incoming.get(*key)).cloned().collect() }
    #[must_use]
    pub fn shadowed_db_keys(&self) -> Vec<String> { self.yaml_values.keys().filter(|key| self.shadows_db_value(key)).cloned().collect() }
    #[must_use]
    pub fn shadows_db_value(&self, key: &str) -> bool { self.owned_by_config(key) && self.db_value_inner(key).is_some_and(|value| Some(value) != self.config_value(key) && Some(value) != self.get(key)) }

    fn keys(&self) -> BTreeSet<String> { self.yaml_values.keys().chain(self.database_rows.values().flat_map(BTreeMap::keys)).chain(self.runtime_values.keys()).cloned().collect() }
    fn resolution_for(&self, key: &str) -> Resolved { resolve(self.yaml_values.get(key), self.db_value_inner(key).filter(|value| !value.is_null())) }
    fn db_value_inner(&self, key: &str) -> Option<&Value> { let row = db_row_for(self.section.as_str(), key); self.database_rows.get(&Section::from(row)).and_then(|values| values.get(key)) }
    fn resolved_value(&self, key: &str) -> Option<&Value> { match self.resolution_for(key).value { SettingValue::Value(_) => self.yaml_values.get(key).or_else(|| self.db_value_inner(key)), SettingValue::Absent => None } }
    fn ownership_error(&self, key: &str) -> ConfigOwnedKeyError { let shadows = self.shadows_db_value(key); ConfigOwnedKeyError { section: self.section.as_str().to_owned(), key: key.to_owned(), shadows_db_value: shadows, message: config_ownership_message(self.section.as_str(), key, shadows) } }
    fn is_resource_list(&self, key: &str) -> bool { self.section == Section::GeneralSettings && key == "pass_through_endpoints" }
}
