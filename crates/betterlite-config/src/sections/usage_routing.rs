use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

// RoutingArgs(**routing_strategy_args) reads one key, the ttl both usage strategies pass to their cache writes.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    #[serde(default)]
    routing_strategy_args: Option<RawArgs>,
}

#[derive(Debug, Default, Deserialize)]
struct RawArgs {
    #[serde(default)]
    ttl: Option<RawNumber>,
}

// the configured ttl; None keeps the one minute RoutingArgs declares
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub ttl_seconds: Option<i64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(args) = self.router_settings.routing_strategy_args else {
            return Ok(Settings::default());
        };
        Ok(Settings {
            ttl_seconds: resolve_number(
                args.ttl,
                "router_settings.routing_strategy_args.ttl",
                lookup,
                integer,
            )?,
        })
    }
}

// pydantic reads an int field from a whole number or a numeric string and refuses the rest
fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
