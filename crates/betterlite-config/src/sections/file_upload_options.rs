use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    require_managed_files: Option<JsonValue>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub require_managed_files: bool,
}

impl Section for Raw {
    type Settings = Settings;

    // litellm.require_managed_files is tested with `is True`, so only the literal JSON true enables the gate
    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            require_managed_files: matches!(
                self.litellm_settings.require_managed_files,
                Some(JsonValue::Bool(true))
            ),
        })
    }
}

mod tests;
