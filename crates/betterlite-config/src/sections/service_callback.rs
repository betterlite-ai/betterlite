use std::str::FromStr;
use std::time::Duration;

use betterlite_db::database_diagnostics::level::{LogLevel, parse_level};
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const LOG_LEVEL_VARIABLE: &str = "LITELLM_LOG";
const JSON_LOGS_VARIABLE: &str = "JSON_LOGS";
const MAX_QUEUE_SIZE_VARIABLE: &str = "LOGGING_WORKER_MAX_QUEUE_SIZE";
const CONCURRENCY_VARIABLE: &str = "LOGGING_WORKER_CONCURRENCY";
const EVENT_TIMEOUT_VARIABLE: &str = "LOGGING_WORKER_MAX_TIME_PER_COROUTINE";

// LiteLLM's _logging module reads LITELLM_LOG with this default
pub const DEFAULT_LOG_LEVEL: &str = "DEBUG";
// LiteLLM constants.py names the three worker limits these variables override
pub const DEFAULT_MAX_QUEUE_SIZE: usize = 50_000;
pub const DEFAULT_CONCURRENCY: usize = 100;
pub const DEFAULT_EVENT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default)]
    service_callback: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum LogLevelError {
    #[error("LITELLM_LOG={value:?} is not a logging level name")]
    Unknown { value: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsValues {
    pub service_callbacks: Vec<String>,
    // the raw LITELLM_LOG text, since Python's logging module resolves the name when it is used
    pub log_level: String,
    pub json_logs: bool,
    pub max_queue_size: usize,
    pub concurrency: usize,
    pub event_timeout: Duration,
}

impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            service_callbacks: Vec::new(),
            log_level: DEFAULT_LOG_LEVEL.to_owned(),
            json_logs: false,
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            event_timeout: DEFAULT_EVENT_TIMEOUT,
        }
    }
}

// boxed so the loaded Config stays small enough for futures that hold it across an await
pub type Settings = Box<SettingsValues>;

impl SettingsValues {
    // LiteLLM's getattr(logging, log_level.upper()) fails on a name with no logging attribute
    pub fn resolve_log_level(&self) -> Result<LogLevel, LogLevelError> {
        parse_level(&self.log_level).map_err(|_source| LogLevelError::Unknown {
            value: self.log_level.clone(),
        })
    }
}

// Python's logging module numbers for the names LITELLM_LOG accepts
#[must_use]
pub const fn level_number(level: LogLevel) -> i32 {
    match level {
        LogLevel::NotSet => 0,
        LogLevel::Debug => 10,
        LogLevel::Info => 20,
        LogLevel::Warning => 30,
        LogLevel::Error => 40,
        LogLevel::Critical => 50,
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Box::new(SettingsValues {
            service_callbacks: resolve_names(
                self.litellm_settings.service_callback.unwrap_or_default(),
                lookup,
            )?,
            // an empty variable counts as unset, matching how betterlite reads every environment value
            log_level: lookup
                .lookup(LOG_LEVEL_VARIABLE)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned()),
            json_logs: json_logs(lookup),
            max_queue_size: variable(lookup, MAX_QUEUE_SIZE_VARIABLE)?
                .unwrap_or(DEFAULT_MAX_QUEUE_SIZE),
            concurrency: variable(lookup, CONCURRENCY_VARIABLE)?.unwrap_or(DEFAULT_CONCURRENCY),
            event_timeout: event_timeout(lookup)?,
        }))
    }
}

fn resolve_names(
    names: Vec<String>,
    lookup: &dyn EnvironmentLookup,
) -> Result<Vec<String>, ConfigError> {
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let path = format!("litellm_settings.service_callback.{index}");
            resolve_string(name, &path, lookup)
        })
        .collect()
}

// only "true" in any case enables JSON logs; whitespace and other spellings stay false
fn json_logs<L>(lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    lookup
        .lookup(JSON_LOGS_VARIABLE)
        .is_some_and(|text| text.to_lowercase() == "true")
}

fn variable<T>(lookup: &dyn EnvironmentLookup, name: &str) -> Result<Option<T>, ConfigError>
where
    T: FromStr,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(None);
    };
    Ok(betterlite_env::number::<T>(name, &text)?)
}

fn event_timeout(lookup: &dyn EnvironmentLookup) -> Result<Duration, ConfigError> {
    let Some(seconds) = variable::<f64>(lookup, EVENT_TIMEOUT_VARIABLE)? else {
        return Ok(DEFAULT_EVENT_TIMEOUT);
    };
    Duration::try_from_secs_f64(seconds).map_err(|_source| ConfigError::InvalidScalar {
        path: EVENT_TIMEOUT_VARIABLE.to_owned(),
        value: false,
    })
}

#[path = "service_callback/tests.rs"]
mod tests;
