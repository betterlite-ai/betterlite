use std::collections::BTreeMap;
use std::num::FpCategory;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

// A key that is present with a null value differs from an absent key: LiteLLM sets None, or an empty list, on the module.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Entry<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Value(T),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<IgnoredAny>),
    Mapping(BTreeMap<String, Self>),
    Null(()),
}

#[derive(Debug, Default, Deserialize)]
struct RawSettingsKeys {
    #[serde(default)]
    user_url_validation: Entry<RawFlag>,
    #[serde(default)]
    user_url_allowed_hosts: Entry<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettingsKeys,
    #[serde(default)]
    general_settings: RawSettingsKeys,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub user_url_validation: bool,
    pub user_url_allowed_hosts: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            user_url_validation: true,
            user_url_allowed_hosts: Vec::new(),
        }
    }
}

// LiteLLM reads the raw module value by Python's bool(), after a string reference is resolved from the environment
fn keeps_validation_on<L>(flag: RawFlag, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(match flag {
        RawFlag::Boolean(value) => value,
        RawFlag::Integer(value) => value != 0,
        RawFlag::Float(value) => value.classify() != FpCategory::Zero,
        RawFlag::Text(text) => !resolve_string(text, path, lookup)?.is_empty(),
        RawFlag::List(items) => !items.is_empty(),
        RawFlag::Mapping(entries) => !entries.is_empty(),
        RawFlag::Null(()) => false,
    })
}

fn replace_hosts<L>(
    entry: Entry<Vec<String>>,
    current: Vec<String>,
    path: &str,
    lookup: &L,
) -> Result<Vec<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match entry {
        Entry::Absent => Ok(current),
        Entry::Null(()) => Ok(Vec::new()),
        Entry::Value(hosts) => hosts
            .into_iter()
            .map(|host| resolve_string(host, path, lookup))
            .collect(),
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let litellm = self.litellm_settings;
        let general = self.general_settings;
        let mut validation = match litellm.user_url_validation {
            Entry::Absent => true,
            Entry::Null(()) => false,
            Entry::Value(flag) => {
                keeps_validation_on(flag, "litellm_settings.user_url_validation", lookup)?
            }
        };
        let path = "general_settings.user_url_validation";
        match general.user_url_validation {
            Entry::Absent | Entry::Null(()) => {}
            // LiteLLM's str_to_bool: anything but true or false leaves the setting alone
            Entry::Value(RawFlag::Text(text)) => {
                match resolve_string(text, path, lookup)?
                    .trim()
                    .to_lowercase()
                    .as_str()
                {
                    "true" => validation = true,
                    "false" => validation = false,
                    _ => {}
                }
            }
            Entry::Value(flag) => validation = keeps_validation_on(flag, path, lookup)?,
        }
        let hosts = replace_hosts(
            litellm.user_url_allowed_hosts,
            Vec::new(),
            "litellm_settings.user_url_allowed_hosts",
            lookup,
        )?;
        Ok(Settings {
            user_url_validation: validation,
            user_url_allowed_hosts: replace_hosts(
                general.user_url_allowed_hosts,
                hosts,
                "general_settings.user_url_allowed_hosts",
                lookup,
            )?,
        })
    }
}
