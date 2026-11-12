use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[path = "vertex_schema_shaping/tests.rs"]
mod vertex_schema_settings;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
const DEFAULT_MAX_DEPTH: i64 = 100;
// LiteLLM's interpreter raises RecursionError near a thousand frames, and a deeper schema would outgrow a worker thread's stack here
const DEPTH_CAP: i64 = 500;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub max_recurse_depth: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_recurse_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let max_recurse_depth = match lookup.lookup(MAX_DEPTH_VARIABLE) {
            None => DEFAULT_MAX_DEPTH,
            Some(text) => betterlite_env::number::<i64>(MAX_DEPTH_VARIABLE, &text)?
                .unwrap_or(DEFAULT_MAX_DEPTH),
        };
        Ok(Settings {
            max_recurse_depth: max_recurse_depth.min(DEPTH_CAP),
        })
    }
}
