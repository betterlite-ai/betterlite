use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;
use crate::types::RoutingStrategy;

mod tests;

const BUFFER_PATH: &str = "router_settings.routing_strategy_args.lowest_latency_buffer";
const PERCENTILE_PATH: &str = "router_settings.routing_strategy_args.ttft_percentile";

// RoutingArgs: the two lowest-latency arguments a selection reads, declared under router_settings.routing_strategy_args
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
    lowest_latency_buffer: Option<RawArg>,
    ttft_percentile: Option<RawArg>,
}

// a number is the argument and any other shape is an error that names the field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawArg {
    Number(RawNumber),
    Other(IgnoredAny),
}

// the configured selection arguments; None keeps the default the handler declares
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LatencyRoutingSettings {
    pub lowest_latency_buffer: Option<f64>,
    pub ttft_percentile: Option<f64>,
}

// boxed so the section adds a pointer to Sections, which sits at clippy's large_futures limit
pub type Settings = Box<LatencyRoutingSettings>;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // Router._build_strategy_selector reads these arguments only for the latency handler
        if self.router_settings.routing_strategy != Some(RoutingStrategy::LatencyBasedRouting) {
            return Ok(Settings::default());
        }
        let Some(args) = self.router_settings.routing_strategy_args else {
            return Ok(Settings::default());
        };
        let buffer = argument(args.lowest_latency_buffer, BUFFER_PATH, lookup)?;
        let percentile = argument(args.ttft_percentile, PERCENTILE_PATH, lookup)?;
        // Field(gt=0, le=1) rejects a fraction outside the interval, NaN and an infinity
        if percentile.is_some_and(|value| !value.is_finite() || value <= 0.0_f64 || value > 1.0_f64)
        {
            return Err(ConfigError::InvalidScalar {
                path: PERCENTILE_PATH.to_owned(),
                value: false,
            });
        }
        Ok(Box::new(LatencyRoutingSettings {
            lowest_latency_buffer: buffer,
            ttft_percentile: percentile,
        }))
    }
}

// a number reaches the field's parser and any other shape is a load error that names the field
fn argument(
    value: Option<RawArg>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<f64>, ConfigError> {
    match value {
        None => Ok(None),
        Some(RawArg::Other(_ignored)) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
        Some(RawArg::Number(number)) => resolve_number(Some(number), path, lookup, parse_float),
    }
}
