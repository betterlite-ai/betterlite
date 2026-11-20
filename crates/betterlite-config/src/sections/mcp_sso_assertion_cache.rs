use std::collections::BTreeMap;

use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const TTL_VARIABLE: &str = "MCP_SSO_ASSERTION_CACHE_TTL_SECONDS";
pub const DEFAULT_TTL_SECONDS: u64 = 60;
pub const MAX_SIZE_VARIABLE: &str = "MCP_OAUTH2_TOKEN_CACHE_MAX_SIZE";
pub const DEFAULT_MAX_SIZE: usize = 200;

// env-only section: the loader reports every key, and the box keeps the map value type non-zero-sized for clippy
pub type Raw = BTreeMap<String, Box<IgnoredAny>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub ttl_seconds: u64,
    pub max_size: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_TTL_SECONDS,
            max_size: DEFAULT_MAX_SIZE,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // An unset variable reads as empty text, and the shared parser treats empty as unset.
        let ttl_text = lookup.lookup(TTL_VARIABLE).unwrap_or_default();
        let ttl_seconds =
            betterlite_env::number::<u64>(TTL_VARIABLE, &ttl_text)?.unwrap_or(DEFAULT_TTL_SECONDS);
        let size_text = lookup.lookup(MAX_SIZE_VARIABLE).unwrap_or_default();
        let max_size = betterlite_env::number::<usize>(MAX_SIZE_VARIABLE, &size_text)?
            .unwrap_or(DEFAULT_MAX_SIZE);
        Ok(Settings {
            ttl_seconds,
            max_size,
        })
    }
}
