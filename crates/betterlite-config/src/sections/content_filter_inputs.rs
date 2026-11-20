use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const RECURSE_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
pub const DEFAULT_RECURSE_DEPTH: i64 = 100;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _marker: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub max_recurse_depth: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_recurse_depth: DEFAULT_RECURSE_DEPTH,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            max_recurse_depth: depth_from_env(lookup)?,
        })
    }
}

// LiteLLM calls int() at import, where an empty value raises; the shared env rule counts empty as unset
fn depth_from_env<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(RECURSE_DEPTH_VARIABLE) else {
        return Ok(DEFAULT_RECURSE_DEPTH);
    };
    let depth = betterlite_env::number::<i64>(RECURSE_DEPTH_VARIABLE, &text)?;
    Ok(depth.unwrap_or(DEFAULT_RECURSE_DEPTH))
}
