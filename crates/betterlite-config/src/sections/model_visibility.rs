mod tests;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const HEALTHY_ONLY_PATH: &str = "general_settings.model_list_healthy_only";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    model_list_healthy_only: Option<RawSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSetting {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

// model_list_healthy_only filters every model listing; only a real true turns it on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub model_list_healthy_only: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            model_list_healthy_only: healthy_only(
                self.general_settings.model_list_healthy_only,
                lookup,
            )?,
        })
    }
}

// LiteLLM reads the setting with `is True`, so no other YAML value turns it on; an os.environ/ reference is read as the flag it names.
fn healthy_only<L>(value: Option<RawSetting>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None | Some(RawSetting::Other(_)) => Ok(false),
        Some(RawSetting::Boolean(enabled)) => Ok(enabled),
        Some(RawSetting::Text(text)) => {
            let variable = text.strip_prefix("os.environ/").map(str::to_owned);
            let resolved = resolve_string(text, HEALTHY_ONLY_PATH, lookup)?;
            let Some(name) = variable else {
                return Ok(false);
            };
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}
