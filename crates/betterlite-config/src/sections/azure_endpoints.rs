use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

pub const DEFAULT_MAX_RETRIES: i64 = 2;
pub const AZURE_DEFAULT_API_VERSION: &str = "2025-02-01-preview";
const MAX_RETRIES_ENV: &str = "DEFAULT_MAX_RETRIES";
const API_VERSION_ENV: &str = "AZURE_DEFAULT_API_VERSION";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default, rename = "DEFAULT_MAX_RETRIES")]
    default_max_retries: Option<RawNumber>,
    #[serde(default, rename = "AZURE_DEFAULT_API_VERSION")]
    azure_default_api_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub default_max_retries: i64,
    pub azure_default_api_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_max_retries: DEFAULT_MAX_RETRIES,
            azure_default_api_version: AZURE_DEFAULT_API_VERSION.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = self.litellm_settings;
        let default_max_retries = resolve_number(
            module.default_max_retries,
            "litellm_settings.DEFAULT_MAX_RETRIES",
            lookup,
            parse_retries,
        )?
        .or(environment_retries(lookup)?)
        .unwrap_or(DEFAULT_MAX_RETRIES);
        let azure_default_api_version = match module.azure_default_api_version {
            Some(text) => {
                resolve_string(text, "litellm_settings.AZURE_DEFAULT_API_VERSION", lookup)?
            }
            None => environment_api_version(lookup),
        };
        Ok(Settings {
            default_max_retries,
            azure_default_api_version,
        })
    }
}

// LiteLLM reads the constant with int(os.getenv(...)), so an empty value counts as unset
fn environment_retries<L>(lookup: &L) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let value = lookup
        .lookup(MAX_RETRIES_ENV)
        .map(|text| betterlite_env::number::<i64>(MAX_RETRIES_ENV, &text))
        .transpose()?
        .flatten();
    Ok(value)
}

// the module setting has no environment default in LiteLLM, and an explicit empty value stays empty
fn environment_api_version<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    lookup
        .lookup(API_VERSION_ENV)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| AZURE_DEFAULT_API_VERSION.to_owned())
}

fn parse_retries(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.trim()
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
