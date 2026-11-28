use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const PATH: &str = "litellm_settings.prometheus_initialize_budget_metrics";
const INTERVAL_VARIABLE: &str = "PROMETHEUS_BUDGET_METRICS_REFRESH_INTERVAL_MINUTES";
const ENV_MARKER: &str = "os.environ/";
pub const DEFAULT_INTERVAL_MINUTES: i64 = 5;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default, rename = "litellm_settings")]
    settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default, rename = "prometheus_initialize_budget_metrics")]
    initialize_budget_metrics: Option<RawToggle>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawToggle {
    Literal(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub initialize_budget_metrics: bool,
    pub refresh_interval_minutes: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            initialize_budget_metrics: false,
            refresh_interval_minutes: DEFAULT_INTERVAL_MINUTES,
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
            initialize_budget_metrics: toggle(self.settings.initialize_budget_metrics, lookup)?,
            refresh_interval_minutes: interval(lookup)?,
        })
    }
}

// LiteLLM's litellm.prometheus_initialize_budget_metrics is Optional[bool] = False and the startup check is `is True`
fn toggle<L>(raw: Option<RawToggle>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        None => Ok(false),
        Some(RawToggle::Literal(value)) => Ok(value),
        Some(RawToggle::Text(text)) => {
            let Some(name) = text.strip_prefix(ENV_MARKER) else {
                return Err(ConfigError::InvalidScalar {
                    path: PATH.to_owned(),
                    value: false,
                });
            };
            let variable = name.to_owned();
            let resolved = resolve_string(text, PATH, lookup)?;
            Ok(betterlite_env::flag(&variable, &resolved)?.unwrap_or(false))
        }
        Some(RawToggle::Other(_)) => Err(ConfigError::InvalidScalar {
            path: PATH.to_owned(),
            value: false,
        }),
    }
}

// bounded below by one: an interval of zero would make the scheduler spin
fn interval<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(INTERVAL_VARIABLE) else {
        return Ok(DEFAULT_INTERVAL_MINUTES);
    };
    let parsed = betterlite_env::number::<i64>(INTERVAL_VARIABLE, &text)?;
    let minutes = parsed.unwrap_or(DEFAULT_INTERVAL_MINUTES);
    if minutes < 1 {
        return Err(ConfigError::InvalidScalar {
            path: INTERVAL_VARIABLE.to_owned(),
            value: false,
        });
    }
    Ok(minutes)
}
