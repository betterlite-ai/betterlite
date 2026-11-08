use std::str::FromStr as _;
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
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
    max_in_flight: Option<RawSetting>,
    #[serde(rename = "max_queued_requests_per_worker")]
    max_queued: Option<RawSetting>,
    // an explicit null reaches LiteLLM's float adapter as None and fails, unlike an absent key
    #[serde(rename = "admission_queue_timeout_seconds", default)]
    queue_timeout: RawSetting,
}

// LiteLLM's _hashable turns every other JSON shape into a repr, so a list or map reaches the same failed check.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawSetting {
    Integer(i64),
    Float(f64),
    Text(String),
    Other(IgnoredAny),
    #[default]
    #[serde(skip_deserializing)]
    Absent,
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
    // an os.environ lookup returned None; LiteLLM stores that None and disables admission for the in-flight key
    EnvironmentUnset,
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
        let max_in_flight = scalar(general.max_in_flight.unwrap_or_default(), lookup);
        if matches!(max_in_flight, Scalar::Unset | Scalar::EnvironmentUnset) {
            return Ok(Settings::default());
        }
        let Some(max_in_flight_requests) = positive_integer(&max_in_flight) else {
            return Ok(invalid(MAX_IN_FLIGHT));
        };
        let configured_queue = scalar(general.max_queued.unwrap_or_default(), lookup);
        let Some(max_queued_requests) = max_queued(&configured_queue, max_in_flight_requests)
        else {
            return Ok(invalid(MAX_QUEUED));
        };
        let configured_timeout = scalar(general.queue_timeout, lookup);
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
        Scalar::Unset | Scalar::EnvironmentUnset => Some(max_in_flight),
        Scalar::Invalid => None,
        Scalar::Integer(_) | Scalar::Float(_) | Scalar::Text(_) => non_negative_integer(configured),
    }
}

fn queue_timeout(configured: &Scalar) -> Option<Duration> {
    match configured {
        Scalar::Unset => Some(Duration::from_secs(1)),
        Scalar::EnvironmentUnset | Scalar::Invalid => None,
        Scalar::Integer(_) | Scalar::Float(_) | Scalar::Text(_) => positive_seconds(configured),
    }
}

fn scalar(raw: RawSetting, lookup: &dyn EnvironmentLookup) -> Scalar {
    match raw {
        RawSetting::Integer(number) => Scalar::Integer(number),
        RawSetting::Float(number) => Scalar::Float(number),
        RawSetting::Other(_) => Scalar::Invalid,
        RawSetting::Absent => Scalar::Unset,
        RawSetting::Text(text) => {
            let Some(variable) = text.strip_prefix("os.environ/").map(str::to_owned) else {
                return Scalar::Text(text);
            };
            let Some(resolved) = lookup.lookup(&variable) else {
                return Scalar::EnvironmentUnset;
            };
            Scalar::Text(resolved)
        }
    }
}

fn integer(scalar: &Scalar) -> Option<i64> {
    match scalar {
        Scalar::Integer(number) => Some(*number),
        Scalar::Float(number) => integer_float(*number),
        Scalar::Text(text) => parse_integer(text),
        Scalar::Unset | Scalar::EnvironmentUnset | Scalar::Invalid => None,
    }
}

fn integer_float(number: f64) -> Option<i64> {
    if !number.is_finite() {
        return None;
    }
    // fract of a whole number is +0.0 even when the number is -0.0 or negative
    if number.fract().to_bits() != 0.0_f64.to_bits() {
        return None;
    }
    format!("{number:.0}").parse::<i64>().ok()
}

// pydantic's int from a string takes a sign, digits with single underscores, and a fraction of zeros only
fn parse_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (integral, fraction) = match trimmed.split_once('.') {
        Some((integral, fraction)) => (integral, Some(fraction)),
        None => (trimmed, None),
    };
    if fraction.is_some_and(|rest| rest.is_empty() || !rest.bytes().all(|byte| byte == b'0')) {
        return None;
    }
    let digits = integral.strip_prefix(['+', '-']).unwrap_or(integral);
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    i64::from_str(&integral.replace('_', "")).ok()
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
        Scalar::Text(text) => {
            let literal = text.trim();
            if literal.starts_with('_') || literal.ends_with('_') || literal.contains("__") {
                return None;
            }
            f64::from_str(&literal.replace('_', "")).ok()?
        }
        Scalar::Unset | Scalar::EnvironmentUnset | Scalar::Invalid => return None,
    };
    if seconds.is_nan() || seconds <= 0.0_f64 {
        return None;
    }
    Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
}
