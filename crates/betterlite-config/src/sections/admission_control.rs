use std::str::FromStr as _;
use std::time::Duration;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const MAX_IN_FLIGHT: &str = "max_in_flight_requests_per_worker";
const MAX_QUEUED: &str = "max_queued_requests_per_worker";
const QUEUE_TIMEOUT: &str = "admission_queue_timeout_seconds";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "max_in_flight_requests_per_worker")]
    max_in_flight: Option<RawNumber>,
    #[serde(rename = "max_queued_requests_per_worker")]
    max_queued: Option<RawNumber>,
    #[serde(rename = "admission_queue_timeout_seconds")]
    queue_timeout: Option<RawNumber>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub limits: Option<Limits>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_in_flight_requests: usize,
    pub max_queued_requests: usize,
    pub queue_timeout: Duration,
}

// LiteLLM reads the raw value, so a string, a float and an int all reach the same positive-number check.
enum Scalar {
    Unset,
    Invalid,
    Integer(i64),
    Float(f64),
    Text(String),
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let max_in_flight = scalar(general.max_in_flight, MAX_IN_FLIGHT, lookup)?;
        if matches!(max_in_flight, Scalar::Unset) {
            return Ok(Settings::default());
        }
        let Some(max_in_flight_requests) = positive_integer(&max_in_flight) else {
            return Ok(invalid(MAX_IN_FLIGHT));
        };
        let configured_queue = scalar(general.max_queued, MAX_QUEUED, lookup)?;
        let Some(max_queued_requests) = max_queued(&configured_queue, max_in_flight_requests)
        else {
            return Ok(invalid(MAX_QUEUED));
        };
        let configured_timeout = scalar(general.queue_timeout, QUEUE_TIMEOUT, lookup)?;
        let Some(queue_timeout) = queue_timeout(&configured_timeout) else {
            return Ok(invalid(QUEUE_TIMEOUT));
        };
        Ok(Settings {
            limits: Some(Limits {
                max_in_flight_requests,
                max_queued_requests,
                queue_timeout,
            }),
        })
    }
}

fn invalid(setting: &str) -> Settings {
    tracing::warn!(
        setting,
        "invalid admission control setting; per-worker admission control is disabled"
    );
    Settings::default()
}

fn max_queued(configured: &Scalar, max_in_flight: usize) -> Option<usize> {
    match configured {
        Scalar::Unset => Some(max_in_flight),
        Scalar::Invalid => None,
        Scalar::Integer(_) | Scalar::Float(_) | Scalar::Text(_) => non_negative_integer(configured),
    }
}

fn queue_timeout(configured: &Scalar) -> Option<Duration> {
    match configured {
        Scalar::Unset => Some(Duration::from_secs(1)),
        Scalar::Invalid => None,
        Scalar::Integer(_) | Scalar::Float(_) | Scalar::Text(_) => positive_seconds(configured),
    }
}

fn scalar(
    raw: Option<RawNumber>,
    name: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Scalar, ConfigError> {
    let Some(configured) = raw else {
        return Ok(Scalar::Unset);
    };
    match configured {
        RawNumber::Integer(number) => Ok(Scalar::Integer(number)),
        RawNumber::Float(number) => Ok(Scalar::Float(number)),
        RawNumber::Boolean(_) => Ok(Scalar::Invalid),
        RawNumber::Text(text) => {
            let Some(variable) = text.strip_prefix("os.environ/").map(str::to_owned) else {
                return Ok(Scalar::Text(text));
            };
            let resolved = resolve_string(text, &format!("general_settings.{name}"), lookup)?;
            let trimmed = resolved.trim();
            if trimmed.is_empty() {
                return Ok(Scalar::Unset);
            }
            match betterlite_env::number::<f64>(&variable, trimmed) {
                Ok(Some(number)) => Ok(Scalar::Float(number)),
                Ok(None) => Ok(Scalar::Unset),
                Err(_) => Ok(Scalar::Invalid),
            }
        }
    }
}

fn integer(scalar: &Scalar) -> Option<i64> {
    match scalar {
        Scalar::Integer(number) => Some(*number),
        Scalar::Float(number) => integer_float(*number),
        Scalar::Text(text) => parse_integer(text),
        Scalar::Unset | Scalar::Invalid => None,
    }
}

fn integer_float(number: f64) -> Option<i64> {
    if !number.is_finite() {
        return None;
    }
    let fraction = number.fract().to_bits();
    if fraction != 0.0_f64.to_bits() && fraction != (-0.0_f64).to_bits() {
        return None;
    }
    format!("{number:.0}").parse::<i64>().ok()
}

fn parse_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if let Ok(number) = i64::from_str(trimmed) {
        return Some(number);
    }
    integer_float(f64::from_str(trimmed).ok()?)
}

fn positive_integer(scalar: &Scalar) -> Option<usize> {
    let number = integer(scalar)?;
    if number <= 0 {
        return None;
    }
    usize::try_from(number).ok()
}

fn non_negative_integer(scalar: &Scalar) -> Option<usize> {
    let number = integer(scalar)?;
    if number < 0 {
        return None;
    }
    usize::try_from(number).ok()
}

fn positive_seconds(scalar: &Scalar) -> Option<Duration> {
    let seconds = match scalar {
        Scalar::Integer(number) => f64::from_str(&number.to_string()).ok()?,
        Scalar::Float(number) => *number,
        Scalar::Text(text) => f64::from_str(text.trim()).ok()?,
        Scalar::Unset | Scalar::Invalid => return None,
    };
    if seconds.is_nan() || seconds <= 0.0_f64 {
        return None;
    }
    if seconds.is_finite() {
        Duration::try_from_secs_f64(seconds).ok()
    } else {
        Some(Duration::MAX)
    }
}
