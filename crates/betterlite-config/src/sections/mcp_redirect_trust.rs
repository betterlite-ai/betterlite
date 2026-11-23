use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const ORIGINS_VARIABLE: &str = "MCP_TRUSTED_REDIRECT_ORIGINS";
pub const NATIVE_VARIABLE: &str = "MCP_TRUSTED_NATIVE_REDIRECT_URIS";

// env-only section: the loader reports every key, and the box keeps the map value type non-zero-sized for clippy
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct Raw(pub BTreeMap<String, Box<IgnoredAny>>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub trusted_origins: String,
    pub native_redirect_uris: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM reads each variable with an empty default and parses the text on every redirect check
        Ok(Settings {
            trusted_origins: lookup.lookup(ORIGINS_VARIABLE).unwrap_or_default(),
            native_redirect_uris: lookup.lookup(NATIVE_VARIABLE).unwrap_or_default(),
        })
    }
}
