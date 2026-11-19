use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number};
use crate::sections::Section;
use crate::types::RoutingStrategy;

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
    routing_strategy: Option<RoutingStrategy>,
    #[serde(default)]
    routing_strategy_args: Option<RawArgs>,
}

// the three keys keep their LiteLLM names in the YAML while the fields say which threshold they hold
#[derive(Debug, Default, Deserialize)]
struct RawArgs {
    #[serde(default, rename = "confidence_threshold_low")]
    low: Option<RawThreshold>,
    #[serde(default, rename = "confidence_threshold_medium")]
    medium: Option<RawThreshold>,
    #[serde(default, rename = "confidence_threshold_high")]
    high: Option<RawThreshold>,
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
        // apply_lar1_routing_strategy reads routing_strategy_args only when the strategy is lar1
        if self.router_settings.routing_strategy != Some(RoutingStrategy::Lar1) {
            return Ok(Settings::default());
        }
        let Some(args) = self.router_settings.routing_strategy_args else {
            return Ok(Settings::default());
        };
        Ok(Settings {
            args: Some(Args {
                low: threshold(
                    args.low,
                    "router_settings.routing_strategy_args.confidence_threshold_low",
                    lookup,
                )?,
                medium: threshold(
                    args.medium,
                    "router_settings.routing_strategy_args.confidence_threshold_medium",
                    lookup,
                )?,
                high: threshold(
                    args.high,
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
        // _coerce_threshold defaults every string; an os.environ/ reference resolves here instead
        Some(RawThreshold::Number(RawNumber::Text(text))) if text.starts_with("os.environ/") => {
            resolve_number(Some(RawNumber::Text(text)), path, lookup, parse_money)
        }
        Some(RawThreshold::Number(RawNumber::Integer(integer))) => Ok(Some(Decimal::from(integer))),
        Some(RawThreshold::Number(RawNumber::Float(float))) => {
            let parsed = Decimal::from_str_exact(&float.to_string());
            parsed.map_or_else(
                |_source| {
                    Err(ConfigError::InvalidScalar {
                        path: path.to_owned(),
                        value: false,
                    })
                },
                |decimal| Ok(Some(decimal)),
            )
        }
        None
        | Some(
            RawThreshold::Other(_)
            | RawThreshold::Number(RawNumber::Boolean(_) | RawNumber::Text(_)),
        ) => Ok(None),
    }
}
