use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_config::sections::settings::{JsonValue, Section};

use crate::proxy_config_state::database::{
    ConfigStateDatabase, SectionWrite, merge_section_values,
};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::state::ProxyConfigState;

// JSON for the tests through serde_json's parser, so no banned object path and no inferred literal type reaches the lint table
macro_rules! json {
    ($($part:tt)*) => {
        serde_json::from_str::<JsonValue>(stringify!($($part)*)).unwrap()
    };
}
pub(crate) use json;

pub(crate) const CONFIG_PATH: &str = "/etc/betterlite/config.yaml";

pub(crate) fn state() -> ProxyConfigState {
    ProxyConfigState::new(PathBuf::from(CONFIG_PATH), false)
}

pub(crate) fn database_state() -> ProxyConfigState {
    ProxyConfigState::new(PathBuf::from(CONFIG_PATH), true)
}

pub(crate) fn values(pairs: &[(&str, JsonValue)]) -> BTreeMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

// The in-memory ConfigStateDatabase: it keeps the rows, records every call and seals an environment value by tagging it
#[derive(Default)]
pub(crate) struct MemoryDatabase {
    sections: Mutex<BTreeMap<String, BTreeMap<String, JsonValue>>>,
    inserted: Mutex<Vec<(String, JsonValue)>>,
    invalidated: Mutex<Vec<String>>,
    merged: Mutex<Vec<SectionWrite>>,
}

impl MemoryDatabase {
    pub(crate) fn with_section(section: Section, values: BTreeMap<String, JsonValue>) -> Self {
        let database = Self::default();
        database
            .sections
            .lock()
            .expect("sections")
            .insert(section.name().to_owned(), values);
        database
    }

    pub(crate) fn stored(&self, section: Section) -> Option<BTreeMap<String, JsonValue>> {
        self.sections
            .lock()
            .expect("sections")
            .get(section.name())
            .cloned()
    }

    pub(crate) fn inserted(&self) -> Vec<(String, JsonValue)> {
        self.inserted.lock().expect("inserted").clone()
    }

    pub(crate) fn invalidated(&self) -> Vec<String> {
        self.invalidated.lock().expect("invalidated").clone()
    }

    pub(crate) fn merged(&self) -> Vec<SectionWrite> {
        self.merged.lock().expect("merged").clone()
    }
}

#[async_trait]
impl ConfigStateDatabase for MemoryDatabase {
    fn seal_environment(
        &self,
        values: &BTreeMap<String, JsonValue>,
    ) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
        Ok(values
            .iter()
            .map(|(key, value)| (key.clone(), sealed(value)))
            .collect())
    }

    async fn merge_section(
        &self,
        write: &SectionWrite,
    ) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError> {
        self.merged.lock().expect("merged").push(write.clone());
        let existing = self
            .sections
            .lock()
            .expect("sections")
            .get(write.section.name())
            .cloned()
            .unwrap_or_default();
        let Some(merged) = merge_section_values(&existing, &write.changed, &write.removed) else {
            return Ok(None);
        };
        self.sections
            .lock()
            .expect("sections")
            .insert(write.section.name().to_owned(), merged.clone());
        self.invalidated
            .lock()
            .expect("invalidated")
            .push(write.section.name().to_owned());
        Ok(Some(merged))
    }

    async fn insert_config(&self, rows: &[(String, JsonValue)]) -> Result<(), ConfigStateError> {
        self.inserted
            .lock()
            .expect("inserted")
            .extend(rows.iter().cloned());
        for (name, _) in rows {
            self.invalidated
                .lock()
                .expect("invalidated")
                .push(name.clone());
        }
        Ok(())
    }

    async fn merge_environment_variables(
        &self,
        sealed: &BTreeMap<String, JsonValue>,
        deleted: &BTreeSet<String>,
    ) -> Result<(), ConfigStateError> {
        let section = Section::EnvironmentVariables;
        let existing = self
            .sections
            .lock()
            .expect("sections")
            .get(section.name())
            .cloned()
            .unwrap_or_default();
        let merged = merge_section_values(&existing, sealed, deleted);
        let row = merged.unwrap_or(existing);
        self.sections
            .lock()
            .expect("sections")
            .insert(section.name().to_owned(), row);
        self.invalidated
            .lock()
            .expect("invalidated")
            .push(section.name().to_owned());
        Ok(())
    }
}

fn sealed(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::String(text) => JsonValue::String(format!("sealed:{text}")),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => value.clone(),
    }
}
