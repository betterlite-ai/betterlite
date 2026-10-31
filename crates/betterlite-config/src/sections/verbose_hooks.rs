#[path = "verbose_hooks/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
#[expect(
    clippy::disallowed_types,
    reason = "The logging hook receives the original JSON truthiness and boolean identity."
)]
type SettingValue = serde_json::Value;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, serde::Deserialize)]
pub struct Global {
    #[serde(default = "default_set_verbose")]
    pub set_verbose: SettingValue,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            set_verbose: default_set_verbose(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub set_verbose: SettingValue,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            set_verbose: resolve_setting(self.litellm_settings.set_verbose, lookup)?,
        })
    }
}

const fn default_set_verbose() -> SettingValue {
    SettingValue::Bool(false)
}

fn resolve_setting(
    value: SettingValue,
    lookup: &impl EnvironmentLookup,
) -> Result<SettingValue, ConfigError> {
    let SettingValue::String(text) = value else {
        return Ok(value);
    };
    let Some(name) = text.strip_prefix("os.environ/") else {
        return Ok(SettingValue::String(text));
    };
    lookup
        .lookup(name)
        .map(SettingValue::String)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path: "litellm_settings.set_verbose".to_owned(),
        })
}
