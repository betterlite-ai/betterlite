use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::database_startup::startup::StartupSettings;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    // an absent key and an explicit null both fall back to the environment
    #[serde(default)]
    disable_prisma_schema_update: Option<DisableUpdate>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Settings,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DisableUpdate {
    Boolean(bool),
    Text(String),
}

// The loader reads YAML, and a JSON scalar is a YAML scalar with its quoting already decided
fn config_source(disable: Option<&DisableUpdate>) -> String {
    let Some(disable) = disable else {
        return "general_settings: {}\n".to_owned();
    };
    let rendered = match disable {
        DisableUpdate::Boolean(value) => value.to_string(),
        DisableUpdate::Text(value) => serde_json::Value::String(value.clone()).to_string(),
    };
    format!("general_settings:\n  disable_prisma_schema_update: {rendered}\n")
}

fn settings_answer(input: &Input) -> HelperResult {
    let config = ConfigLoader {
        lookup: input.environment.clone(),
    }
    .load(&config_source(input.disable_prisma_schema_update.as_ref()))
    .or_config_error()?;
    let settings = StartupSettings::from_config(&config);
    Ok(serde_json::json!({
        "updates_enabled": settings.updates_enabled,
        "command_timeout": settings.command_timeout.as_secs_f64(),
        "lock_timeout": settings.lock_timeout.as_secs_f64(),
        "deploy_timeout": settings.deploy_timeout.as_secs_f64(),
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Settings => settings_answer(input),
        }
    }
}
