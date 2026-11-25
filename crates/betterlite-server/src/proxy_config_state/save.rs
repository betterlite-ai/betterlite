use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::{DbRow, JsonValue, Section, changed_section_keys};
use tokio::fs;

use super::database::{ConfigStateDatabase, SectionWrite};
use super::error::ConfigStateError;
use super::snapshot::ConfigWithBaseline;
use super::state::ProxyConfigState;

// _CONFIG_PERSISTED_SECTIONS: the sections that live in their own config row
const PERSISTED_SECTIONS: [Section; 3] = [
    Section::GeneralSettings,
    Section::RouterSettings,
    Section::LitellmSettings,
];

// _CONFIG_UNMANAGED_EXCLUSIONS: rows another path owns, so a top-level key of that name is never inserted here
const UNMANAGED_EXCLUSIONS: [&str; 2] = ["environment_variables", "model_list"];

const ENVIRONMENT_VARIABLES: &str = "environment_variables";

// save_config: a database plus store_model_in_db persists the changed sections, anything else rewrites the file
pub async fn save_config(
    state: &mut ProxyConfigState,
    new_config: &mut ConfigWithBaseline,
    include_env_vars: bool,
    database: Option<&dyn ConfigStateDatabase>,
) -> Result<(), ConfigStateError> {
    let Some(backed) = database.filter(|_| state.store_model_in_db_enabled()) else {
        return write_config_file(state, &new_config.values).await;
    };
    save_config_to_database(state, new_config, include_env_vars, backed).await
}

async fn save_config_to_database(
    state: &mut ProxyConfigState,
    new_config: &mut ConfigWithBaseline,
    include_env_vars: bool,
    database: &dyn ConfigStateDatabase,
) -> Result<(), ConfigStateError> {
    let keeps_baseline = new_config.baseline.is_some();
    let baseline = new_config
        .baseline
        .clone()
        .unwrap_or_else(|| state.snapshot());
    for section in PERSISTED_SECTIONS {
        save_changed_section(state, section, &baseline, &new_config.values, database).await?;
    }
    let unmanaged = unmanaged_values(&baseline, &new_config.values);
    if !unmanaged.is_empty() {
        database.insert_config(&unmanaged).await?;
    }
    if include_env_vars {
        save_environment_section(&new_config.values, database).await?;
    }
    let next_config = merged_config(&baseline, &new_config.values);
    if keeps_baseline {
        new_config.baseline = Some(next_config.clone());
    }
    state.replace_snapshot(&next_config);
    Ok(())
}

async fn save_changed_section(
    state: &mut ProxyConfigState,
    section: Section,
    baseline: &BTreeMap<String, JsonValue>,
    new_config: &BTreeMap<String, JsonValue>,
    database: &dyn ConfigStateDatabase,
) -> Result<(), ConfigStateError> {
    let Some(new_value) = new_config.get(section.name()) else {
        return Ok(());
    };
    let new_section = section_values(section, new_value)?;
    let baseline_section = match baseline.get(section.name()) {
        Some(value) => section_values(section, value)?,
        None => BTreeMap::new(),
    };
    let changes = changed_section_keys::changed_section_keys(&baseline_section, &new_section);
    state.reject_config_owned_writes(section, &changes.changed)?;
    if changes.changed.is_empty() && changes.removed.is_empty() {
        return Ok(());
    }
    let write = SectionWrite {
        section,
        changed: changes.changed,
        removed: changes.removed,
    };
    let Some(written) = database.merge_section(&write).await? else {
        return Ok(());
    };
    state.apply_section_row(section, DbRow::from(section), written);
    Ok(())
}

// save_environment_variables: a named key is written or, for None, deleted, and every other key stays
pub async fn save_environment_variables(
    state: &ProxyConfigState,
    updates: &BTreeMap<String, Option<String>>,
    database: Option<&dyn ConfigStateDatabase>,
) -> Result<(), ConfigStateError> {
    let Some(backed) = database.filter(|_| state.store_model_in_db_enabled()) else {
        return Ok(());
    };
    let mut values = BTreeMap::new();
    let mut deleted = BTreeSet::new();
    for (key, value) in updates {
        match value {
            Some(text) => {
                values.insert(key.clone(), JsonValue::String(text.clone()));
            }
            None => {
                deleted.insert(key.clone());
            }
        }
    }
    let sealed = if values.is_empty() {
        values
    } else {
        backed.seal_environment(&values)?
    };
    backed.merge_environment_variables(&sealed, &deleted).await
}

async fn write_config_file(
    state: &ProxyConfigState,
    config: &BTreeMap<String, JsonValue>,
) -> Result<(), ConfigStateError> {
    let yaml = serde_saphyr::to_string(config)
        .map_err(|source| ConfigStateError::SerializeConfig { source })?;
    fs::write(&state.config_file_path, yaml)
        .await
        .map_err(|source| ConfigStateError::WriteConfigFile {
            path: state.config_file_path.clone(),
            source,
        })
}

async fn save_environment_section(
    new_config: &BTreeMap<String, JsonValue>,
    database: &dyn ConfigStateDatabase,
) -> Result<(), ConfigStateError> {
    let Some(value) = new_config.get(ENVIRONMENT_VARIABLES) else {
        return Ok(());
    };
    let values: BTreeMap<String, JsonValue> = match value {
        JsonValue::Null => return Ok(()),
        JsonValue::Object(entries) => entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect(),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            return Err(ConfigStateError::InvalidSetting {
                field: ENVIRONMENT_VARIABLES,
                expected: "an object",
            });
        }
    };
    let sealed = if values.is_empty() {
        values
    } else {
        database.seal_environment(&values)?
    };
    let row = JsonValue::Object(sealed.into_iter().collect());
    database
        .insert_config(&[(ENVIRONMENT_VARIABLES.to_owned(), row)])
        .await
}

fn section_values(
    section: Section,
    value: &JsonValue,
) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
    match value {
        JsonValue::Object(entries) => Ok(entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(ConfigStateError::InvalidSetting {
            field: section.name(),
            expected: "an object",
        }),
    }
}

fn unmanaged_values(
    baseline: &BTreeMap<String, JsonValue>,
    new_config: &BTreeMap<String, JsonValue>,
) -> Vec<(String, JsonValue)> {
    new_config
        .iter()
        .filter(|(key, value)| {
            let name = key.as_str();
            !PERSISTED_SECTIONS
                .iter()
                .any(|section| section.name() == name)
                && !UNMANAGED_EXCLUSIONS.contains(&name)
                && baseline.get(name) != Some(value)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn merged_config(
    baseline: &BTreeMap<String, JsonValue>,
    new_config: &BTreeMap<String, JsonValue>,
) -> BTreeMap<String, JsonValue> {
    let mut merged = baseline.clone();
    merged.extend(new_config.clone());
    merged
}
