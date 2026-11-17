use rust_decimal::Decimal;
use thiserror::Error;

// DEFAULT_THRESHOLDS, the thresholds Router uses when routing_strategy_args names none
const DEFAULT_LOW: Decimal = Decimal::from_parts(3, 0, 0, false, 1);
const DEFAULT_MEDIUM: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
const DEFAULT_HIGH: Decimal = Decimal::from_parts(7, 0, 0, false, 1);

// the confidence_below → deployment type boundaries, normalized once at startup
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub low: Decimal,
    pub medium: Decimal,
    pub high: Decimal,
}

// confidence_threshold_low/medium/high as the configuration resolved them; None keeps the default
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThresholdArgs {
    pub low: Option<Decimal>,
    pub medium: Option<Decimal>,
    pub high: Option<Decimal>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThresholdError {
    #[error(
        "LAR-1 thresholds must satisfy 0 < low < medium < high < 1, got low={low}, medium={medium}, high={high}"
    )]
    Order {
        low: Decimal,
        medium: Decimal,
        high: Decimal,
    },
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            low: DEFAULT_LOW,
            medium: DEFAULT_MEDIUM,
            high: DEFAULT_HIGH,
        }
    }
}

impl Thresholds {
    // _normalize_thresholds: merge the defaults, then refuse an order no confidence can fall through
    pub fn from_args(args: Option<&ThresholdArgs>) -> Result<Self, ThresholdError> {
        let defaults = Self::default();
        let configured = args.copied().unwrap_or_default();
        let thresholds = Self {
            low: configured.low.unwrap_or(defaults.low),
            medium: configured.medium.unwrap_or(defaults.medium),
            high: configured.high.unwrap_or(defaults.high),
        };
        let ordered = Decimal::ZERO < thresholds.low
            && thresholds.low < thresholds.medium
            && thresholds.medium < thresholds.high
            && thresholds.high < Decimal::ONE;
        if !ordered {
            return Err(ThresholdError::Order {
                low: thresholds.low,
                medium: thresholds.medium,
                high: thresholds.high,
            });
        }
        Ok(thresholds)
    }
}
