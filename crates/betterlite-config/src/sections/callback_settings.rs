use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::env_integer::get_env_int;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const CAPACITY_VARIABLE: &str = "LITELLM_MAX_CALLBACKS";
const DEFAULT_CAPACITY: i64 = 100;
// no exporter is compiled into betterlite yet, so every LiteLLM callback name is unavailable
const INSTALLED_CALLBACKS: [&str; 0] = [];

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    callback_settings: BTreeMap<String, IndexMap<String, RawValue>>,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(default)]
    callbacks: Vec<String>,
    #[serde(default)]
    success_callback: Vec<String>,
    #[serde(default)]
    failure_callback: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Text(String),
    Integer(i64),
    Flag(bool),
    Other(IgnoredAny),
}

impl RawValue {
    fn text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Integer(number) => Some(number.to_string()),
            Self::Flag(set) => Some(if set { "True" } else { "False" }.to_owned()),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub callbacks: Vec<String>,
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callback_settings: BTreeMap<String, IndexMap<String, String>>,
    pub max_callbacks: usize,
}

fn unavailable_paths(lists: &RawLiteLlm) -> Vec<String> {
    [
        ("litellm_settings.callbacks", &lists.callbacks),
        ("litellm_settings.success_callback", &lists.success_callback),
        ("litellm_settings.failure_callback", &lists.failure_callback),
    ]
    .into_iter()
    .filter(|(_, names)| {
        names
            .iter()
            .any(|name| !INSTALLED_CALLBACKS.contains(&name.as_str()))
    })
    .map(|(path, _)| path.to_owned())
    .collect()
}

fn text_variables(entry: IndexMap<String, RawValue>) -> IndexMap<String, String> {
    entry
        .into_iter()
        .filter_map(|(key, value)| value.text().map(|text| (key, text)))
        .collect()
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw_capacity = get_env_int(
            lookup.lookup(CAPACITY_VARIABLE).as_deref(),
            DEFAULT_CAPACITY,
        );
        // a negative or oversized limit admits nothing, exactly as LiteLLM's negative limit skips every add
        let max_callbacks = usize::try_from(raw_capacity).unwrap_or_default();
        let paths = unavailable_paths(&self.litellm_settings);
        if !paths.is_empty() {
            return Err(ConfigError::Unsupported { paths });
        }
        let callback_settings = self
            .callback_settings
            .into_iter()
            .map(|(name, entry)| (name, text_variables(entry)))
            .collect();
        Ok(Settings {
            callbacks: self.litellm_settings.callbacks,
            success_callback: self.litellm_settings.success_callback,
            failure_callback: self.litellm_settings.failure_callback,
            callback_settings,
            max_callbacks,
        })
    }
}
