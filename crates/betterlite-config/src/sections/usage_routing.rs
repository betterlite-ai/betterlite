use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_number;
use crate::sections::Section;

mod tests;

const TTL_PATH: &str = "router_settings.routing_strategy_args.ttl";

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
    ttl: Option<RawTtl>,
}

// RoutingArgs holds an int: a number is the ttl and any other argument is a load error that names the field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTtl {
    Number(RawNumber),
    Other(IgnoredAny),
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
        let Some(ttl) = args.ttl else {
            return Ok(Settings::default());
        };
        let RawTtl::Number(number) = ttl else {
            return Err(ConfigError::InvalidScalar {
                path: TTL_PATH.to_owned(),
                value: false,
            });
        };
        Ok(Settings {
            ttl_seconds: resolve_number(Some(number), TTL_PATH, lookup, integer)?,
        })
    }
}

// the shared key holds whole non-negative seconds, so a negative ttl is the error the unsigned parse reports
fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    let seconds = text
        .parse::<u64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })?;
    match i64::try_from(seconds) {
        Ok(fitted) => Ok(fitted),
        Err(_overflow) => text
            .parse::<i64>()
            .map_err(|source| ConfigError::InvalidInteger {
                path: path.to_owned(),
                source,
            }),
    }
}
