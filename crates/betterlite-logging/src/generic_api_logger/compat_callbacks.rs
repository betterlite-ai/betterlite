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
    #[error("the environment placeholder pattern is not valid")]
    Pattern(#[source] regex::Error),
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
}
#[must_use]
pub fn is_callback_compatible(callbacks: &CompatibleCallbacks, name: &str) -> bool {
    // the caller parsed the file once, so a broken data file already failed in embedded()
    callbacks.configs.contains_key(name)
}
#[must_use]
pub fn get_callback_config(callbacks: &CompatibleCallbacks, name: &str) -> Option<CallbackConfig> {
    // only a name the parsed map lacks gives None here
    callbacks.configs.get(name).cloned()
}
pub fn substitute_env_variables(
    value: &str,
    environment: &dyn EnvironmentLookup,
) -> Result<String, CompatError> {
    let pattern = Regex::new(PLACEHOLDER).map_err(CompatError::Pattern)?;
    let replaced = pattern
        .replace_all(value, |captures: &Captures<'_>| {
            let name = captures.get(1).map_or("", |matched| matched.as_str());
            environment.lookup(name).unwrap_or_default()
        })
        .into_owned();
    Ok(replaced)
}
