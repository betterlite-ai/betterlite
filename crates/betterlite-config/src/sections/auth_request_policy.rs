use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const REQUEST_SIZE_SETTING: &str = "general_settings.max_request_size_mb";
const RESPONSE_SIZE_SETTING: &str = "general_settings.max_response_size_mb";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    allowed_ips: Option<Vec<String>>,
    #[serde(default)]
    use_x_forwarded_for: Option<bool>,
    #[serde(default)]
    max_request_size_mb: Option<serde_json::Number>,
    #[serde(default)]
    max_response_size_mb: Option<serde_json::Number>,
    #[serde(default)]
    allowed_routes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub allowed_ips: Option<Vec<String>>,
    pub use_x_forwarded_for: bool,
    pub max_request_size_mb: Option<f64>,
    pub max_response_size_mb: Option<f64>,
    pub allowed_routes: Option<Vec<String>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general_settings = self.general_settings;
        Ok(Settings {
            allowed_ips: general_settings.allowed_ips,
            use_x_forwarded_for: general_settings.use_x_forwarded_for.unwrap_or_default(),
            max_request_size_mb: megabytes(
                general_settings.max_request_size_mb,
                REQUEST_SIZE_SETTING,
            )?,
            max_response_size_mb: megabytes(
                general_settings.max_response_size_mb,
                RESPONSE_SIZE_SETTING,
            )?,
            allowed_routes: general_settings.allowed_routes,
        })
    }
}

// LiteLLM declares an int and takes any number, so an integer and a float both load
fn megabytes(raw: Option<serde_json::Number>, path: &str) -> Result<Option<f64>, ConfigError> {
    let Some(number) = raw else {
        return Ok(None);
    };
    let value = number
        .to_string()
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: path.to_owned(),
            source,
        })?;
    Ok(Some(value))
}
