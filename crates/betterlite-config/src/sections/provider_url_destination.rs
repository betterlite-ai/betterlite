use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::provider_url_destination::pattern::AllowedDestination;

pub mod pattern;
mod tests;

const LITELLM_PATH: &str = "litellm_settings.provider_url_destination_allowed_hosts";
const GENERAL_PATH: &str = "general_settings.provider_url_destination_allowed_hosts";

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

// litellm_settings is set raw on the litellm module, and the matcher accepts a bare string as one entry
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Hosts {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Default, Deserialize)]
struct LiteLlmKeys {
    #[serde(default, rename = "provider_url_destination_allowed_hosts")]
    allowed_hosts: Entry<Hosts>,
}

#[derive(Debug, Default, Deserialize)]
struct GeneralKeys {
    #[serde(default, rename = "provider_url_destination_allowed_hosts")]
    allowed_hosts: Entry<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: LiteLlmKeys,
    #[serde(default)]
    general_settings: GeneralKeys,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub allowed_hosts: Vec<AllowedDestination>,
}

fn resolve_entries<L>(
    entries: Vec<String>,
    path: &str,
    lookup: &L,
) -> Result<Vec<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    entries
        .into_iter()
        .map(|entry| resolve_string(entry, path, lookup))
        .collect()
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut hosts = match self.litellm_settings.allowed_hosts {
            Entry::Absent | Entry::Null(()) => Vec::new(),
            Entry::Value(Hosts::One(host)) => resolve_entries(vec![host], LITELLM_PATH, lookup)?,
            Entry::Value(Hosts::Many(list)) => resolve_entries(list, LITELLM_PATH, lookup)?,
        };
        hosts = match self.general_settings.allowed_hosts {
            Entry::Absent => hosts,
            Entry::Null(()) => Vec::new(),
            Entry::Value(list) => resolve_entries(list, GENERAL_PATH, lookup)?,
        };
        Ok(Settings {
            allowed_hosts: hosts
                .iter()
                .filter_map(|host| AllowedDestination::parse(host))
                .collect(),
        })
    }
}
