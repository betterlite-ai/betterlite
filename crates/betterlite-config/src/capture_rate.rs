use std::collections::BTreeMap;

use betterlite_models::capture_rate::settings::{
    SpendCaptureProvider, SpendCaptureRateCheckSettings,
};
use betterlite_models::capture_rate::{DEFAULT_LOOKBACK_DAYS, DEFAULT_THRESHOLD, MAX_RANGE_DAYS};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number};

mod tests;

const THRESHOLD_PATH: &str = "general_settings.spend_capture_rate_check.threshold";
const LOOKBACK_PATH: &str = "general_settings.spend_capture_rate_check.lookback_days";

// ProxyConfig reads general_settings.spend_capture_rate_check as SpendCaptureRateCheckSettings, absent by default.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    general_settings: Option<GeneralSettings>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GeneralSettings {
    spend_capture_rate_check: Option<RawCheck>,
}

// SpendCaptureRateCheckSettings has extra="forbid"; every field keeps its pydantic default when absent or null.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawCheck {
    providers: Option<Vec<SpendCaptureProvider>>,
    threshold: Option<RawNumber>,
    lookback_days: Option<RawNumber>,
    openai_project_ids: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureRateSettingsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("general_settings.spend_capture_rate_check.providers must list at least one provider")]
    EmptyProviders,
    #[error(
        "general_settings.spend_capture_rate_check.threshold must be greater than 0 and at most 1, got {threshold}"
    )]
    ThresholdOutOfRange { threshold: Decimal },
    #[error(
        "general_settings.spend_capture_rate_check.lookback_days must be between 1 and 180, got {lookback_days}"
    )]
    LookbackOutOfRange { lookback_days: i64 },
}

pub fn load_capture_rate_settings<L>(
    source: &str,
    lookup: &L,
) -> Result<Option<SpendCaptureRateCheckSettings>, CaptureRateSettingsError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    raw.convert(lookup)
}

pub fn decode_capture_rate_settings(
    source: &str,
) -> Result<Option<SpendCaptureRateCheckSettings>, CaptureRateSettingsError> {
    match parse_view::<Raw>(source) {
        Ok((raw, _ignored)) => raw.convert(&BTreeMap::<String, String>::new()),
        Err(error) => Err(CaptureRateSettingsError::Config(error)),
    }
}

impl Raw {
    fn convert<L>(
        self,
        lookup: &L,
    ) -> Result<Option<SpendCaptureRateCheckSettings>, CaptureRateSettingsError>
    where
        L: EnvironmentLookup,
    {
        let Some(check) = self
            .general_settings
            .and_then(|general| general.spend_capture_rate_check)
        else {
            return Ok(None);
        };
        let providers = check
            .providers
            .unwrap_or_else(|| vec![SpendCaptureProvider::Openai]);
        if providers.is_empty() {
            return Err(CaptureRateSettingsError::EmptyProviders);
        }
        let threshold = resolve_number(check.threshold, THRESHOLD_PATH, lookup, parse_money)?
            .unwrap_or(DEFAULT_THRESHOLD);
        if threshold <= Decimal::ZERO || threshold > Decimal::ONE {
            return Err(CaptureRateSettingsError::ThresholdOutOfRange { threshold });
        }
        let lookback_days = resolve_number(check.lookback_days, LOOKBACK_PATH, lookup, parse_days)?
            .unwrap_or(DEFAULT_LOOKBACK_DAYS);
        if !(1..=MAX_RANGE_DAYS).contains(&lookback_days) {
            return Err(CaptureRateSettingsError::LookbackOutOfRange { lookback_days });
        }
        Ok(Some(SpendCaptureRateCheckSettings {
            providers,
            threshold,
            lookback_days,
            openai_project_ids: check.openai_project_ids.unwrap_or_default(),
        }))
    }
}

fn parse_days(text: &str, path: &str) -> Result<i64, ConfigError> {
    match text.parse::<i64>() {
        Ok(value) => Ok(value),
        Err(source) => Err(ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        }),
    }
}
