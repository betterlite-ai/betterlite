use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;
use serde::Deserialize;
mod tests;
// DEFAULT_POLLING_INTERVAL: the scheduler's poll cadence when nothing sets one
const DEFAULT_POLLING_INTERVAL: f64 = 0.03;
// DEFAULT_IN_MEMORY_TTL: how long the in-memory queue copy lives beside a Redis cache
const DEFAULT_IN_MEMORY_TTL: f64 = 5.0;
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}
#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    polling_interval: Option<RawNumber>,
    default_priority: Option<RawNumber>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub polling_interval: f64,
    pub default_priority: Option<u8>,
    pub local_ttl_seconds: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            polling_interval: DEFAULT_POLLING_INTERVAL,
            default_priority: None,
            local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
        }
    }
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = resolve_number(
            self.router_settings.polling_interval,
            "router_settings.polling_interval",
            lookup,
            parse_float,
        )?;
        // Scheduler.__init__ resolves `polling_interval or DEFAULT_POLLING_INTERVAL`, so a configured zero takes the default
        let fallback =
            environment_number("DEFAULT_POLLING_INTERVAL", DEFAULT_POLLING_INTERVAL, lookup)?;
        let polling_interval = match configured {
            Some(seconds) if seconds != 0.0 => seconds,
            Some(_) | None => fallback,
        };
        let default_priority = resolve_number(
            self.router_settings.default_priority,
            "router_settings.default_priority",
            lookup,
            parse_priority,
        )?;
        let local_ttl_seconds =
            environment_number("DEFAULT_IN_MEMORY_TTL", DEFAULT_IN_MEMORY_TTL, lookup)?;
        Ok(Settings {
            polling_interval,
            default_priority,
            local_ttl_seconds,
        })
    }
}
fn environment_number<L>(name: &str, fallback: f64, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(fallback);
    };
    Ok(betterlite_env::number::<f64>(name, &text)?.unwrap_or(fallback))
}
// FlowItem documents priorities between 0 and 255, and the queue stores them as a byte
fn parse_priority(text: &str, path: &str) -> Result<u8, ConfigError> {
    text.parse::<u8>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
