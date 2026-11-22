use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const INCLUDE_COST_PATH: &str = "litellm_settings.include_cost_in_streaming_usage";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default, rename = "include_cost_in_streaming_usage")]
    cost_in_streaming_usage: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

// LiteLLM's module global is off until a config sets it, and a null value leaves it off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            enabled: configured_flag(
                self.litellm_settings.cost_in_streaming_usage,
                INCLUDE_COST_PATH,
                lookup,
            )?,
        })
    }
}

fn configured_flag<L>(value: Option<RawFlag>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(flag)) => Ok(flag),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}

mod tests;
