use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;

const DEFAULT_LIMIT_MB: f64 = 50.0;
const VARIABLE: &str = "MAX_IMAGE_URL_DOWNLOAD_SIZE_MB";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub max_download_size_mb: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_download_size_mb: DEFAULT_LIMIT_MB,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = lookup.lookup(VARIABLE).unwrap_or_default();
        // Python's float() reads decimal text, inf, infinity and nan; anything else fails the import
        let max_download_size_mb = match configured.trim() {
            "" => DEFAULT_LIMIT_MB,
            text => text
                .parse::<f64>()
                .map_err(|source| ConfigError::InvalidFloat {
                    path: VARIABLE.to_owned(),
                    source,
                })?,
        };
        Ok(Settings {
            max_download_size_mb,
        })
    }
}

#[path = "image_url_download/tests.rs"]
mod tests;
