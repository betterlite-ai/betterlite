use betterlite_env::EnvError;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    turn_off_message_logging: Option<Flag>,
    redact_user_api_key_info: Option<Flag>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub turn_off_message_logging: bool,
    pub redact_user_api_key_info: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            turn_off_message_logging: resolve(
                self.litellm_settings.turn_off_message_logging,
                "turn_off_message_logging",
                lookup,
            )?,
            redact_user_api_key_info: resolve(
                self.litellm_settings.redact_user_api_key_info,
                "redact_user_api_key_info",
                lookup,
            )?,
        })
    }
}

#[path = "logging_privacy/tests.rs"]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Boolean(bool),
    Environment(String),
}

fn resolve<L>(value: Option<Flag>, name: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(Flag::Boolean(flag)) => Ok(flag),
        Some(Flag::Environment(reference)) => {
            let path = format!("litellm_settings.{name}");
            if !reference.starts_with("os.environ/") {
                return Err(ConfigError::Environment {
                    source: EnvError::Bool { name: path },
                });
            }
            let text = resolve_string(reference, &path, lookup)?;
            Ok(betterlite_env::flag(&path, &text)?.unwrap_or(false))
        }
    }
}
