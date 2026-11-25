use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::proxy_config_state::error::ConfigStateError;
use betterlite_server::proxy_config_state::state::ProxyConfigState;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("unknown b8380 operation: {0}")]
    UnknownOperation(String),
    #[error("{0} is not a settings section")]
    UnknownSection(String),
    #[error(transparent)]
    Config(#[from] ConfigStateError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: String,
    section: String,
    #[serde(default)]
    yaml_values: BTreeMap<String, JsonValue>,
    #[serde(default)]
    database_row: Option<BTreeMap<String, JsonValue>>,
    changed_keys: BTreeMap<String, JsonValue>,
    config_file_path: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

// reject_config_owned_writes: None when the store lets the write through, otherwise LiteLLM's 400 detail
fn answer(input: &Input) -> Result<Value, Failure> {
    if input.operation != "owned_writes" {
        return Err(Failure::UnknownOperation(input.operation.clone()));
    }
    let section = section_of(&input.section)?;
    let mut state = ProxyConfigState::new(PathBuf::from(&input.config_file_path), false);
    let Some(store) = store_mut(&mut state, section) else {
        return Ok(json!({"status_code": null, "detail": null}));
    };
    store.load_yaml(input.yaml_values.clone());
    if let Some(row) = &input.database_row {
        store.apply_db_row(DbRow::from(section), row.clone());
    }
    match state.reject_config_owned_writes(section, &input.changed_keys) {
        Ok(()) => Ok(json!({"status_code": null, "detail": null})),
        Err(ConfigStateError::ConfigOwned(detail)) => Ok(json!({
            "status_code": 400_u16,
            "detail": detail,
        })),
        Err(other) => Err(Failure::Config(other)),
    }
}

fn section_of(name: &str) -> Result<Section, Failure> {
    match name {
        "general_settings" => Ok(Section::GeneralSettings),
        "router_settings" => Ok(Section::RouterSettings),
        "litellm_settings" => Ok(Section::LitellmSettings),
        "environment_variables" => Ok(Section::EnvironmentVariables),
        "ui_settings" => Ok(Section::UiSettings),
        other => Err(Failure::UnknownSection(other.to_owned())),
    }
}

const fn store_mut(state: &mut ProxyConfigState, section: Section) -> Option<&mut SettingsStore> {
    match section {
        Section::GeneralSettings => Some(&mut state.general_settings),
        Section::RouterSettings => Some(&mut state.router_settings),
        Section::LitellmSettings => Some(&mut state.litellm_settings),
        Section::EnvironmentVariables => Some(&mut state.environment_variables),
        Section::UiSettings => None,
    }
}
