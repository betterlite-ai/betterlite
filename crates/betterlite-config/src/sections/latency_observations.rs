use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;
use crate::types::RoutingStrategy;

mod tests;

const TTL_PATH: &str = "router_settings.routing_strategy_args.ttl";
const BUFFER_PATH: &str = "router_settings.routing_strategy_args.lowest_latency_buffer";
const SIZE_PATH: &str = "router_settings.routing_strategy_args.max_latency_list_size";
const PERCENTILE_PATH: &str = "router_settings.routing_strategy_args.ttft_percentile";

// RoutingArgs: the lowest-latency handler's arguments, all declared under router_settings.routing_strategy_args
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    #[serde(default)]
    routing_strategy: Option<RoutingStrategy>,
    #[serde(default)]
    routing_strategy_args: Option<RawArgs>,
}

#[derive(Debug, Default, Deserialize)]
struct RawArgs {
    ttl: Option<RawTtl>,
    lowest_latency_buffer: Option<RawNumber>,
    max_latency_list_size: Option<RawNumber>,
    ttft_percentile: Option<RawNumber>,
}

// RoutingArgs.ttl is a float, and any other shape is an error that names the field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTtl {
    Number(RawNumber),
    Other(IgnoredAny),
}

// the configured arguments; None keeps the default the handler declares
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args {
    pub ttl_seconds: Option<f64>,
    pub lowest_latency_buffer: Option<f64>,
    pub max_latency_list_size: Option<i64>,
    pub ttft_percentile: Option<f64>,
}

pub type Settings = Box<Args>;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // Router._build_strategy_selector reads these arguments only for the latency handler
        if self.router_settings.routing_strategy != Some(RoutingStrategy::LatencyBasedRouting) {
            return Ok(Box::default());
        }
        let Some(args) = self.router_settings.routing_strategy_args else {
            return Ok(Box::default());
        };
        let ttl = match args.ttl {
            None => None,
            Some(RawTtl::Other(_ignored)) => {
                return Err(ConfigError::InvalidScalar {
                    path: TTL_PATH.to_owned(),
                    value: false,
                });
            }
            Some(RawTtl::Number(number)) => {
                resolve_number(Some(number), TTL_PATH, lookup, parse_float)?
            }
        };
        let buffer = resolve_number(args.lowest_latency_buffer, BUFFER_PATH, lookup, parse_float)?;
        let size = resolve_number(args.max_latency_list_size, SIZE_PATH, lookup, parse_size)?;
        let percentile =
            resolve_number(args.ttft_percentile, PERCENTILE_PATH, lookup, parse_float)?;
        // Field(gt=0, le=1) rejects a fraction outside the interval, NaN and an infinity
        if percentile.is_some_and(|value| !value.is_finite() || value <= 0.0_f64 || value > 1.0_f64)
        {
            return Err(ConfigError::InvalidScalar {
                path: PERCENTILE_PATH.to_owned(),
                value: false,
            });
        }
        Ok(Box::new(Args {
            ttl_seconds: ttl,
            lowest_latency_buffer: buffer,
            max_latency_list_size: size,
            ttft_percentile: percentile,
        }))
    }
}

// pydantic's int from a number: a whole float is the integer and a fractional part is an error
fn parse_size(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
