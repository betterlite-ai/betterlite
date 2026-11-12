use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const ENABLED_PATH: &str = "general_settings.enable_claude_code_gateway";
const MANAGED_PATH: &str = "general_settings.claude_code_gateway_managed_settings";

pub type ManagedSettings = IndexMap<String, Json>;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    enable_claude_code_gateway: Option<RawFlag>,
    #[serde(rename = "claude_code_gateway_managed_settings")]
    managed_settings: Option<RawManagedSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawManagedSettings {
    Mapping(ManagedSettings),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub managed_settings: Option<ManagedSettings>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            enabled: enabled_value(general.enable_claude_code_gateway, lookup)?,
            managed_settings: managed_value(general.managed_settings)?,
        })
    }
}

fn enabled_value<L>(value: Option<RawFlag>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let invalid = || ConfigError::InvalidScalar {
        path: ENABLED_PATH.to_owned(),
        value: false,
    };
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(flag)) => Ok(flag),
        Some(RawFlag::Text(text)) => {
            let Some(variable) = text.strip_prefix("os.environ/").map(str::to_owned) else {
                return Err(invalid());
            };
            let resolved = resolve_string(text, ENABLED_PATH, lookup)?;
            Ok(betterlite_env::flag(&variable, &resolved)?.unwrap_or(false))
        }
        Some(RawFlag::Other(_)) => Err(invalid()),
    }
}

fn managed_value(
    value: Option<RawManagedSettings>,
) -> Result<Option<ManagedSettings>, ConfigError> {
    match value {
        None => Ok(None),
        Some(RawManagedSettings::Mapping(settings)) => Ok(Some(settings)),
        Some(RawManagedSettings::Other(_)) => Err(ConfigError::InvalidScalar {
            path: MANAGED_PATH.to_owned(),
            value: false,
        }),
    }
}
