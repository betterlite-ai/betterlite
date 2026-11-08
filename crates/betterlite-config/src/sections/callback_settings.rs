use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::env_integer::get_env_int;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub mod names;
mod tests;

const CAPACITY_VARIABLE: &str = "LITELLM_MAX_CALLBACKS";
const DEFAULT_CAPACITY: i64 = 100;
const OTEL_ENTRY: &str = "otel";
// the nested callback_settings.otel keys RawOtel declares, including its aliases; otel_settings parses them
const OTEL_NESTED_KEYS: [&str; 11] = [
    "LITELLM_OTEL_BAGGAGE_METADATA_KEYS",
    "LITELLM_OTEL_BAGGAGE_PROMOTED_KEYS",
    "LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS",
    "LITELLM_OTEL_EXCLUDED_SERVICES",
    "baggage_metadata_keys",
    "baggage_promoted_keys",
    "baggage_team_metadata_keys",
    "excluded_services",
    "exporters",
    "mapper_names",
    "resource_attributes",
];

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
    callbacks: Option<RawCallbackList>,
    #[serde(default)]
    success_callback: Vec<String>,
    #[serde(default)]
    failure_callback: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawCallbackList {
    One(String),
    Many(Vec<String>),
}

impl RawCallbackList {
    fn names(&self) -> Vec<&str> {
        match self {
            Self::One(name) => vec![name.as_str()],
            Self::Many(names) => names.iter().map(String::as_str).collect(),
        }
    }

    fn into_names(self) -> Vec<String> {
        match self {
            Self::One(name) => vec![name],
            Self::Many(names) => names,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Text(String),
    Number(serde_json::Number),
    Flag(bool),
    Null,
    Other(IgnoredAny),
}

impl RawValue {
    fn into_callback_value<L>(
        self,
        path: &str,
        lookup: &L,
    ) -> Result<Option<CallbackValue>, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(match self {
            Self::Text(text) => Some(CallbackValue::Text(SecretString::from(resolve_string(
                text, path, lookup,
            )?))),
            Self::Number(number) => Some(CallbackValue::Number(number.to_string())),
            Self::Flag(set) => Some(CallbackValue::Flag(set)),
            Self::Null => Some(CallbackValue::Null),
            Self::Other(_) => None,
        })
    }
}

#[derive(Clone, Debug)]
enum CallbackValue {
    Flag(bool),
    Null,
    Number(String),
    Text(SecretString),
}

impl CallbackValue {
    fn exposed_text(&self) -> Option<String> {
        match self {
            Self::Text(secret) => Some(secret.expose_secret().to_owned()),
            Self::Number(number) => Some(number.clone()),
            Self::Flag(set) => Some(if *set { "True" } else { "False" }.to_owned()),
            Self::Null => None,
        }
    }
}

// every value stays a secret until the adapter or validator that reads it exposes it
#[derive(Clone, Debug, Default)]
pub struct CallbackEntry {
    values: IndexMap<String, CallbackValue>,
}

impl CallbackEntry {
    #[must_use]
    pub fn exposed_variables(&self) -> IndexMap<String, String> {
        self.values
            .iter()
            .filter_map(|(key, value)| value.exposed_text().map(|text| (key.clone(), text)))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub callbacks: Vec<String>,
    pub success_callback: Vec<String>,
    pub failure_callback: Vec<String>,
    pub callback_settings: BTreeMap<String, CallbackEntry>,
    pub max_callbacks: usize,
}

fn unavailable_paths(lists: &RawLiteLlm) -> Vec<String> {
    let mut paths = Vec::new();
    let callbacks = lists
        .callbacks
        .as_ref()
        .map(RawCallbackList::names)
        .unwrap_or_default();
    if callbacks
        .iter()
        .any(|name| names::CallbackName::from_name(name).is_none())
    {
        paths.push("litellm_settings.callbacks".to_owned());
    }
    for (path, names) in [
        ("litellm_settings.success_callback", &lists.success_callback),
        ("litellm_settings.failure_callback", &lists.failure_callback),
    ] {
        if names
            .iter()
            .any(|name| names::CallbackName::from_name(name).is_none())
        {
            paths.push(path.to_owned());
        }
    }
    paths
}

fn callback_entries<L>(
    raw: BTreeMap<String, IndexMap<String, RawValue>>,
    paths: &mut Vec<String>,
    lookup: &L,
) -> Result<BTreeMap<String, CallbackEntry>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut entries = BTreeMap::new();
    for (name, values) in raw {
        let mut converted = IndexMap::new();
        for (key, value) in values {
            let path = format!("callback_settings.{name}.{key}");
            if let Some(setting) = value.into_callback_value(&path, lookup)? {
                converted.insert(key, setting);
                continue;
            }
            let owned = name == OTEL_ENTRY && OTEL_NESTED_KEYS.contains(&key.as_str());
            if !owned {
                paths.push(path);
            }
        }
        entries.insert(name, CallbackEntry { values: converted });
    }
    Ok(entries)
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
        let mut paths = unavailable_paths(&self.litellm_settings);
        let callback_settings = callback_entries(self.callback_settings, &mut paths, lookup)?;
        if !paths.is_empty() {
            return Err(ConfigError::Unsupported { paths });
        }
        Ok(Settings {
            callbacks: self
                .litellm_settings
                .callbacks
                .map(RawCallbackList::into_names)
                .unwrap_or_default(),
            success_callback: self.litellm_settings.success_callback,
            failure_callback: self.litellm_settings.failure_callback,
            callback_settings,
            max_callbacks,
        })
    }
}
