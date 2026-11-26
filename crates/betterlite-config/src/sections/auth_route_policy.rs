use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    admin_only_routes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub admin_only_routes: Option<Vec<String>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            admin_only_routes: self.general_settings.admin_only_routes,
        })
    }
}
