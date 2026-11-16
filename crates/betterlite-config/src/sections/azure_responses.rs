use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const VERSION_VARIABLE: &str = "AZURE_DEFAULT_RESPONSES_API_VERSION";
pub const DEFAULT_VERSION: &str = "preview";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // LiteLLM reads the version only from the environment, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

// one pointer in Sections: the loader keeps every section's settings on the stack under a deep YAML walk (B9972)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub resolved: Box<Resolved>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub default_responses_api_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resolved: Box::new(Resolved {
                default_responses_api_version: DEFAULT_VERSION.to_owned(),
            }),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            resolved: Box::new(Resolved {
                default_responses_api_version: version(lookup),
            }),
        })
    }
}

// LiteLLM reads the variable with os.getenv; the project's compatibility rules make an empty value unset
fn version<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    match lookup.lookup(VERSION_VARIABLE) {
        Some(value) if !value.is_empty() => value,
        Some(_) | None => DEFAULT_VERSION.to_owned(),
    }
}
