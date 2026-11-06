pub mod clock;

mod tests;

use chrono::NaiveTime;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use clock::ClockError;

const TIMEZONE_PATH: &str = "litellm_settings.timezone";
const RESET_TIME_PATH: &str = "litellm_settings.budget_reset_time";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Global {
    #[serde(default)]
    timezone: JsonValue,
    #[serde(rename = "budget_reset_time", default)]
    reset_time_of_day: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetResetSettings {
    pub timezone: String,
    pub reset_time_of_day: NaiveTime,
}

// the sections! macro names every section's settings type Settings
pub type Settings = BudgetResetSettings;

impl Default for BudgetResetSettings {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_owned(),
            reset_time_of_day: NaiveTime::MIN,
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BudgetResetTimeError {
    #[error("budget_reset_time must be a quoted 24-hour 'HH:MM' string, e.g. \"12:00\"")]
    NotText,
    #[error(transparent)]
    Clock(#[from] ClockError),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BudgetResetSettingsError {
    #[error("litellm_settings.timezone must be a timezone name string")]
    TimezoneNotText,
    #[error(transparent)]
    ResetTime(#[from] BudgetResetTimeError),
}

pub fn get_budget_reset_settings(
    timezone: &JsonValue,
    reset_time_of_day: &JsonValue,
) -> Result<BudgetResetSettings, BudgetResetSettingsError> {
    Ok(BudgetResetSettings {
        timezone: reset_timezone(timezone)?,
        reset_time_of_day: parse_budget_reset_time(reset_time_of_day)?,
    })
}

pub fn parse_budget_reset_time(value: &JsonValue) -> Result<NaiveTime, BudgetResetTimeError> {
    match value {
        JsonValue::Null => Ok(NaiveTime::MIN),
        JsonValue::String(text) if text.is_empty() => Ok(NaiveTime::MIN),
        JsonValue::String(text) => {
            clock::decode_reset_clock(text).map_err(BudgetResetTimeError::from)
        }
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(BudgetResetTimeError::NotText)
        }
    }
}

fn reset_timezone(value: &JsonValue) -> Result<String, BudgetResetSettingsError> {
    match value {
        JsonValue::String(text) if text.is_empty() => Ok("UTC".to_owned()),
        JsonValue::String(text) => Ok(text.clone()),
        JsonValue::Null | JsonValue::Bool(false) => Ok("UTC".to_owned()),
        JsonValue::Number(number) if number.as_f64().is_some_and(|float| float == 0.0_f64) => {
            Ok("UTC".to_owned())
        }
        JsonValue::Array(items) if items.is_empty() => Ok("UTC".to_owned()),
        JsonValue::Object(fields) if fields.is_empty() => Ok("UTC".to_owned()),
        JsonValue::Bool(true)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(BudgetResetSettingsError::TimezoneNotText),
    }
}

impl Section for Raw {
    type Settings = BudgetResetSettings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let timezone = resolve_json(self.litellm_settings.timezone, TIMEZONE_PATH, lookup)?;
        let reset_time = resolve_json(
            self.litellm_settings.reset_time_of_day,
            RESET_TIME_PATH,
            lookup,
        )?;
        let invalid = |path: &str, raw: &JsonValue| ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: matches!(raw, JsonValue::Bool(true)),
        };
        get_budget_reset_settings(&timezone, &reset_time).map_err(|error| match error {
            BudgetResetSettingsError::TimezoneNotText => invalid(TIMEZONE_PATH, &timezone),
            BudgetResetSettingsError::ResetTime(_) => invalid(RESET_TIME_PATH, &reset_time),
        })
    }
}

fn resolve_json<L>(value: JsonValue, path: &str, lookup: &L) -> Result<JsonValue, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        JsonValue::String(text) => resolve_string(text, path, lookup).map(JsonValue::String),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => Ok(other),
    }
}
