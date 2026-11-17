use betterlite_env::EnvError;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    disable_streaming_logging: Option<Flag>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamingLogging {
    pub disable: bool,
}

// the sections! macro names every section's settings type Settings
pub type Settings = StreamingLogging;

impl Section for Raw {
    type Settings = StreamingLogging;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(StreamingLogging {
            disable: resolve(self.litellm_settings.disable_streaming_logging, lookup)?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Boolean(bool),
    Environment(String),
}

fn resolve<L>(value: Option<Flag>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    const PATH: &str = "litellm_settings.disable_streaming_logging";
    match value {
        None => Ok(false),
        Some(Flag::Boolean(flag)) => Ok(flag),
        Some(Flag::Environment(reference)) => {
            if !reference.starts_with("os.environ/") {
                return Err(ConfigError::Environment {
                    source: EnvError::Bool {
                        name: PATH.to_owned(),
                    },
                });
            }
            let text = resolve_string(reference, PATH, lookup)?;
            Ok(betterlite_env::flag(PATH, &text)?.unwrap_or(false))
        }
    }
}
