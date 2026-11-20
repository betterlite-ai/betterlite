mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawGeneral {
    pub key_management_system: Option<JsonValue>,
    pub key_management_settings: Option<JsonValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub key_management_system: Option<String>,
    pub key_management_settings: Option<JsonValue>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            // the early bootstrap resolves an os.environ/ name while no manager exists, so the text is kept here
            key_management_system: general
                .key_management_system
                .as_ref()
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            key_management_settings: general.key_management_settings,
        })
    }
}
