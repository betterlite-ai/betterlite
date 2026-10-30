mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const DEFAULT_MAJOR_VERSION: u32 = 7;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub default_redis_major_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_redis_major_version: DEFAULT_MAJOR_VERSION,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let default_redis_major_version = lookup.lookup("DEFAULT_REDIS_MAJOR_VERSION").map_or(
            Ok(DEFAULT_MAJOR_VERSION),
            |text| {
                text.trim()
                    .parse::<u32>()
                    .map_err(|source| ConfigError::InvalidInteger {
                        path: "DEFAULT_REDIS_MAJOR_VERSION".to_owned(),
                        source,
                    })
            },
        )?;
        Ok(Settings {
            default_redis_major_version,
        })
    }
}
