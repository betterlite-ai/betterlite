use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number};
use crate::sections::Section;

mod tests;

// lar1_thresholds_from_args reads the three arguments under router_settings.routing_strategy_args
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
    confidence_threshold_low: Option<RawThreshold>,
    #[serde(default)]
    confidence_threshold_medium: Option<RawThreshold>,
    #[serde(default)]
    confidence_threshold_high: Option<RawThreshold>,
}

// _coerce_threshold: a number is the threshold and every other argument keeps the declared default
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawThreshold {
    Number(RawNumber),
    Other(IgnoredAny),
}

// the configured arguments; None keeps the default the router declares
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub args: Option<Args>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub low: Option<Decimal>,
    pub medium: Option<Decimal>,
    pub high: Option<Decimal>,
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
            args: Some(Args {
                low: threshold(
                    args.confidence_threshold_low,
                    "router_settings.routing_strategy_args.confidence_threshold_low",
                    lookup,
                )?,
                medium: threshold(
                    args.confidence_threshold_medium,
                    "router_settings.routing_strategy_args.confidence_threshold_medium",
                    lookup,
                )?,
                high: threshold(
                    args.confidence_threshold_high,
                    "router_settings.routing_strategy_args.confidence_threshold_high",
                    lookup,
                )?,
            }),
        })
    }
}

fn threshold(
    value: Option<RawThreshold>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Decimal>, ConfigError> {
    match value {
        None | Some(RawThreshold::Other(_)) => Ok(None),
        Some(RawThreshold::Number(RawNumber::Boolean(_))) => Ok(None),
        Some(RawThreshold::Number(RawNumber::Integer(integer))) => Ok(Some(Decimal::from(integer))),
        Some(RawThreshold::Number(RawNumber::Float(float))) => {
            parse_money(&float.to_string(), path).map(Some)
        }
        // _coerce_threshold defaults every string; an os.environ/ reference resolves here instead
        Some(RawThreshold::Number(RawNumber::Text(text))) if text.starts_with("os.environ/") => {
            resolve_number(Some(RawNumber::Text(text)), path, lookup, parse_money)
        }
        Some(RawThreshold::Number(RawNumber::Text(_))) => Ok(None),
    }
}
