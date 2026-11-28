mod tests;

use std::collections::BTreeMap;

use betterlite_env::number;
use serde::de::IgnoredAny;

use crate::env_integer::timeout::RequestTimeout;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// env-only section: no YAML key is read here, and the newtype keeps this Section impl off other sections' map type
#[derive(Default, serde::Deserialize)]
#[serde(transparent)]
pub struct Raw(pub BTreeMap<String, Box<IgnoredAny>>);

// The module globals the image edit entry points read: the generic timeout fallback.
#[derive(Debug, Clone)]
pub struct Settings {
    pub request_timeout_seconds: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            request_timeout_seconds: RequestTimeout::DEFAULT_SECONDS,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment_text = lookup.lookup("REQUEST_TIMEOUT").unwrap_or_default();
        let request_timeout_seconds = number::<f64>("REQUEST_TIMEOUT", &environment_text)?
            .unwrap_or(RequestTimeout::DEFAULT_SECONDS);
        Ok(Settings {
            request_timeout_seconds,
        })
    }
}
