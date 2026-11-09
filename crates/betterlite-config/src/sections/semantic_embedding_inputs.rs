use betterlite_env;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const EMBEDDING_TIMEOUT_ENV: &str = "SEMANTIC_CACHE_EMBEDDING_TIMEOUT_SECONDS";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 5.0;

// A skipped marker keeps this environment-only section a YAML mapping.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub embedding_timeout_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            embedding_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            embedding_timeout_seconds: embedding_timeout(lookup)?,
        })
    }
}

fn embedding_timeout<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(EMBEDDING_TIMEOUT_ENV) else {
        return Ok(DEFAULT_TIMEOUT_SECONDS);
    };
    let configured = betterlite_env::number::<f64>(EMBEDDING_TIMEOUT_ENV, &text)?;
    Ok(configured.unwrap_or(DEFAULT_TIMEOUT_SECONDS))
}
