use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::settings_resolution::FieldSource;
use super::settings_rules::{DbRow, JsonValue, Section};

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{}", config_ownership_message(*.section, .key, *.shadows_db_value))]
pub struct ConfigOwnedKeyError {
    pub section: Section,
    pub key: String,
    pub shadows_db_value: bool,
}

#[must_use]
pub fn config_ownership_message(section: Section, key: &str, shadows_db_value: bool) -> String {
    let stored = if shadows_db_value {
        " The value stored in the database for it is ignored and will never be applied."
    } else {
        ""
    };
    format!(
        "{}.{key} is set in the config file, so the config file owns it and it cannot be changed here.{stored} Edit the config file to change it, or remove it from the file to let the database own it.",
        section.name()
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution<'store> {
    Config(&'store JsonValue),
    Db(&'store JsonValue),
    Unset,
}

impl Resolution<'_> {
    #[must_use]
    pub const fn source(self) -> FieldSource {
        match self {
            Self::Config(_) => FieldSource::Config,
            Self::Db(_) => FieldSource::Db,
            Self::Unset => FieldSource::Unset,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsStore {
    section: Section,
    yaml_values: BTreeMap<String, JsonValue>,
    database_rows: BTreeMap<DbRow, BTreeMap<String, JsonValue>>,
    runtime_values: BTreeMap<String, JsonValue>,
    deleted_runtime_keys: BTreeSet<String>,
}

impl From<Section> for SettingsStore {
    fn from(section: Section) -> Self {
        Self {
            section,
            yaml_values: BTreeMap::new(),
            database_rows: BTreeMap::new(),
            runtime_values: BTreeMap::new(),
            deleted_runtime_keys: BTreeSet::new(),
        }
    }
}

impl SettingsStore {
    pub fn load_yaml(&mut self, mut values: BTreeMap<String, JsonValue>) {
        let section = self.section;
        values.retain(|key, _| !section.is_resource_list(key));
        self.yaml_values = values;
        self.runtime_values
            .retain(|key, _| section.is_resource_list(key));
        self.deleted_runtime_keys.clear();
    }

    pub fn apply_db_row(&mut self, row: DbRow, values: BTreeMap<String, JsonValue>) {
        let previous = self.database_rows.remove(&row).unwrap_or_default();
        let changed: BTreeSet<String> = previous
            .keys()
            .chain(values.keys())
            .filter(|key| previous.get(*key) != values.get(*key))
            .cloned()
            .collect();
        self.database_rows.insert(row, values);
        for key in changed {
            if !self.yaml_values.contains_key(&key) {
                self.runtime_values.remove(&key);
                self.deleted_runtime_keys.remove(&key);
            }
        }
    }

    pub fn apply_runtime_values(&mut self, values: BTreeMap<String, JsonValue>) {
        self.runtime_values = values;
        self.deleted_runtime_keys.clear();
    }

    #[must_use]
    pub fn resolution(&self, key: &str) -> Resolution<'_> {
        let stored = self
            .stored_value(key)
            .filter(|value| **value != JsonValue::Null);
        self.yaml_values
            .get(key)
            .map(Resolution::Config)
            .or_else(|| stored.map(Resolution::Db))
            .unwrap_or(Resolution::Unset)
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        if self.deleted_runtime_keys.contains(key) {
            None
        } else if let Some(value) = self.runtime_values.get(key) {
            Some(value)
        } else if let Resolution::Config(value) | Resolution::Db(value) = self.resolution(key) {
            Some(value)
        } else {
            None
        }
    }

    pub fn insert(&mut self, key: &str, value: JsonValue) -> Result<(), ConfigOwnedKeyError> {
        if self.yaml_values.contains_key(key) && self.get(key) != Some(&value) {
            return Err(self.ownership_error(key));
        }
        self.runtime_values.insert(key.to_owned(), value);
        self.deleted_runtime_keys.remove(key);
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<Option<JsonValue>, ConfigOwnedKeyError> {
        let Some(previous) = self.get(key).cloned() else {
            return Ok(None);
        };
        if self.yaml_values.contains_key(key) {
            return Err(self.ownership_error(key));
        }
        self.runtime_values.remove(key);
        self.deleted_runtime_keys.insert(key.to_owned());
        Ok(Some(previous))
    }

    pub fn clear(&mut self) {
        let removable: BTreeSet<String> = self
            .candidate_keys()
            .into_iter()
            .filter(|key| !self.yaml_values.contains_key(*key))
            .map(str::to_owned)
            .collect();
        let yaml_values = &self.yaml_values;
        self.runtime_values
            .retain(|key, _| yaml_values.contains_key(key));
        self.deleted_runtime_keys = removable;
    }

    #[must_use]
    pub fn resolved(&self) -> BTreeMap<String, JsonValue> {
        let mut resolved = BTreeMap::new();
        for key in self.candidate_keys() {
            if let Some(value) = self.get(key) {
                resolved.insert(key.to_owned(), value.clone());
            }
        }
        resolved
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &JsonValue)> {
        let keys = self.candidate_keys();
        keys.into_iter()
            .filter_map(|key| self.get(key).map(|value| (key, value)))
    }

    #[must_use]
    pub fn db_value(&self, key: &str) -> Option<&JsonValue> {
        if self.section.is_resource_list(key) {
            self.stored_value(key)
        } else {
            None
        }
    }

    #[must_use]
    pub fn rejected_writes(&self, incoming: &BTreeMap<String, JsonValue>) -> Vec<String> {
        let mut rejected = Vec::new();
        for (key, value) in incoming {
            if self.yaml_values.contains_key(key) && self.get(key) != Some(value) {
                rejected.push(key.clone());
            }
        }
        rejected
    }

    #[must_use]
    pub fn shadowed_db_keys(&self) -> Vec<String> {
        let mut shadowed = Vec::new();
        for key in self.yaml_values.keys() {
            if self.shadows_db_value(key) {
                shadowed.push(key.clone());
            }
        }
        shadowed
    }

    #[must_use]
    pub fn shadows_db_value(&self, key: &str) -> bool {
        let Some(config) = self.yaml_values.get(key) else {
            return false;
        };
        self.stored_value(key).is_some_and(|stored| {
            *stored != JsonValue::Null && Some(stored) != self.get(key) && stored != config
        })
    }

    fn ownership_error(&self, key: &str) -> ConfigOwnedKeyError {
        let shadows_db_value = self.shadows_db_value(key);
        ConfigOwnedKeyError {
            section: self.section,
            key: key.to_owned(),
            shadows_db_value,
        }
    }

    fn stored_value(&self, key: &str) -> Option<&JsonValue> {
        let row = self.section.row_for(key);
        let values = self.database_rows.get(&row)?;
        values.get(key)
    }

    fn candidate_keys(&self) -> BTreeSet<&str> {
        let stored = self.database_rows.values().flat_map(BTreeMap::keys);
        let keys = self
            .yaml_values
            .keys()
            .chain(stored)
            .chain(self.runtime_values.keys());
        keys.map(String::as_str).collect()
    }
}
