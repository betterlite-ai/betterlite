use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

// RoutingArgs.ttl: int = 1 * 60
pub const DEFAULT_TTL_SECONDS: u64 = 60;

const TTL_PATH: &str = "router_settings.routing_strategy_args.ttl";

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
    ttl: Option<RawTtl>,
}

// RoutingArgs holds an int: a number is the ttl and any other argument is a load error that names the field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTtl {
    Number(RawNumber),
    Other(IgnoredAny),
}

// the window ttl the v2 rpm and tpm counters and their cache entries carry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub ttl: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(DEFAULT_TTL_SECONDS),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(ttl) = self
            .router_settings
            .routing_strategy_args
            .and_then(|args| args.ttl)
        else {
            return Ok(Settings::default());
        };
        let number = match ttl {
            RawTtl::Number(number) => number,
            RawTtl::Other(_ignored) => {
                return Err(ConfigError::InvalidScalar {
                    path: TTL_PATH.to_owned(),
                    value: false,
                });
            }
        };
        let Some(seconds) = resolve_number(Some(number), TTL_PATH, lookup, parse_ttl)? else {
            return Ok(Settings::default());
        };
        Ok(Settings {
            ttl: Duration::from_secs(seconds),
        })
    }
}

fn parse_ttl(text: &str, path: &str) -> Result<u64, ConfigError> {
    text.parse::<u64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
