mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(rename = "SERVER_ROOT_PATH")]
    server_root_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub server_root_path: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            server_root_path: self
                .server_root_path
                .or_else(|| lookup.lookup("SERVER_ROOT_PATH"))
                .unwrap_or_default(),
        })
    }
}
