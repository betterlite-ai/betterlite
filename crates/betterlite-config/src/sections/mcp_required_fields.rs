use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;
mod tests;
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}
#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    mcp_required_fields: Option<Vec<String>>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub required_fields: Vec<String>,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            required_fields: self
                .general_settings
                .mcp_required_fields
                .unwrap_or_default(),
        })
    }
}
