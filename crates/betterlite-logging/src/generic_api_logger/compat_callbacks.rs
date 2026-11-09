use betterlite_config::loader::EnvironmentLookup;
use indexmap::IndexMap;
use regex::{Captures, Regex};
use serde::Deserialize;
use std::collections::BTreeMap;
mod tests;
const EMBEDDED: &str = include_str!("compatible_callbacks.json");
const PLACEHOLDER: &str = r"\{\{environment_variables\.([A-Z_]+)\}\}";
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackConfig {
    #[serde(default)]
    pub event_types: Option<Vec<String>>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub log_format: Option<String>,
    #[serde(default)]
    pub environment_variables: Vec<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum CompatError {
    #[error("generic_api_compatible_callbacks.json is not valid")]
    Json(#[source] serde_json::Error),
}
#[derive(Clone, Debug, Default)]
pub struct CompatibleCallbacks {
    configs: BTreeMap<String, CallbackConfig>,
}
impl CompatibleCallbacks {
    pub fn embedded() -> Result<Self, CompatError> {
        let configs: BTreeMap<String, CallbackConfig> =
            serde_json::from_str(EMBEDDED).map_err(CompatError::Json)?;
        Ok(Self { configs })
    }
    pub fn parse(text: &str) -> Result<Self, CompatError> {
        let configs: BTreeMap<String, CallbackConfig> =
            serde_json::from_str(text).map_err(CompatError::Json)?;
        Ok(Self { configs })
    }
}
#[must_use]
pub fn is_callback_compatible(name: &str) -> bool {
    let Ok(callbacks) = CompatibleCallbacks::embedded() else {
        return false;
    };
    callbacks.configs.contains_key(name)
}
#[must_use]
pub fn get_callback_config(name: &str) -> Option<CallbackConfig> {
    let Ok(callbacks) = CompatibleCallbacks::embedded() else {
        return None;
    };
    callbacks.configs.get(name).cloned()
}
#[must_use]
pub fn substitute_env_variables(value: &str, environment: &dyn EnvironmentLookup) -> String {
    let Ok(pattern) = Regex::new(PLACEHOLDER) else {
        return value.to_owned();
    };
    pattern
        .replace_all(value, |captures: &Captures<'_>| {
            let name = captures.get(1).map_or("", |matched| matched.as_str());
            environment.lookup(name).unwrap_or_default()
        })
        .into_owned()
}
