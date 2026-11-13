use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const SETTING: &str = "general_settings.max_request_size_mb";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    max_request_size_mb: Option<serde_json::Number>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Settings {
    pub max_request_size_mb: Option<f64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(number) = self.general_settings.max_request_size_mb else {
            return Ok(Settings::default());
        };
        // LiteLLM declares an int and the middleware takes any number, so an integer and a float both load
        let max_request_size_mb =
            number
                .to_string()
                .parse::<f64>()
                .map_err(|source| ConfigError::InvalidFloat {
                    path: SETTING.to_owned(),
                    source,
                })?;
        Ok(Settings {
            max_request_size_mb: Some(max_request_size_mb),
        })
    }
}
