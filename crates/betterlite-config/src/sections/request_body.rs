use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const VARIABLE: &str = "MAX_REQUEST_BODY_SIZE_TO_REPAIR_MB";
const DEFAULT_MEGABYTES: i64 = 1;
const MEBIBYTE: u64 = 1024 * 1024;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub max_repair_bytes: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_repair_bytes: repair_cap(DEFAULT_MEGABYTES),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(text) = lookup.lookup(VARIABLE) else {
            return Ok(Settings::default());
        };
        let Ok(megabytes) = text.trim().parse::<i64>() else {
            // the pinned proxy falls back to the default on an unusable value instead of failing the load
            tracing::warn!(
                variable = VARIABLE,
                value = text,
                "the repair size limit is not an integer; using 1 MiB"
            );
            return Ok(Settings::default());
        };
        Ok(Settings {
            max_repair_bytes: repair_cap(megabytes),
        })
    }
}

// LiteLLM skips the repair entirely when the limit is zero or negative, so those values carry no cap
fn repair_cap(megabytes: i64) -> Option<u64> {
    let positive = u64::try_from(megabytes).ok().filter(|value| *value > 0);
    positive.and_then(|value| value.checked_mul(MEBIBYTE))
}
