use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const VARIABLE: &str = "GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub ttl_seconds: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { ttl_seconds: 86400 }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = lookup.lookup(VARIABLE).unwrap_or_default();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(Settings::default());
        }
        let ttl_seconds = trimmed
            .parse::<i64>()
            .map_err(|source| ConfigError::InvalidInteger {
                path: VARIABLE.to_owned(),
                source,
            })?;
        Ok(Settings { ttl_seconds })
    }
}
