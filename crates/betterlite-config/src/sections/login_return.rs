use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const CONTROL_PLANE_URL_PATH: &str = "general_settings.control_plane_url";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    control_plane_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub control_plane_url: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let control_plane_url = self
            .general_settings
            .control_plane_url
            .map(|text| resolve_string(text, CONTROL_PLANE_URL_PATH, lookup))
            .transpose()?;
        Ok(Settings { control_plane_url })
    }
}

mod tests;
