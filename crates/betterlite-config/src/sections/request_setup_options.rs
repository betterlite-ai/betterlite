use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const ENVIRON_PREFIX: &str = "os.environ/";
const DISABLING_PATH: &str = "litellm_settings.allow_dynamic_callback_disabling";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(rename = "allow_dynamic_callback_disabling")]
    callback_disabling: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Flag(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub callback_disabling: bool,
}

// litellm/__init__.py ships the switch on, so a config that never names it still honours the header
impl Default for Settings {
    fn default() -> Self {
        Self {
            callback_disabling: true,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = match self.litellm_settings.callback_disabling {
            None => None,
            Some(RawFlag::Flag(flag)) => Some(flag),
            Some(RawFlag::Text(text)) => {
                let Some(name) = text.strip_prefix(ENVIRON_PREFIX) else {
                    return Err(ConfigError::InvalidScalar {
                        path: DISABLING_PATH.to_owned(),
                        value: false,
                    });
                };
                let value = lookup
                    .lookup(name)
                    .ok_or_else(|| ConfigError::MissingEnvironment {
                        variable: name.to_owned(),
                        path: DISABLING_PATH.to_owned(),
                    })?;
                betterlite_env::flag(name, &value)?
            }
            Some(RawFlag::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: DISABLING_PATH.to_owned(),
                    value: false,
                });
            }
        };
        Ok(Settings {
            callback_disabling: configured.unwrap_or(true),
        })
    }
}
