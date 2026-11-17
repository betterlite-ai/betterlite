use std::str::FromStr;
use std::time::Duration;

use betterlite_db::database_diagnostics::level::{LogLevel, parse_level};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

const LOG_LEVEL_VARIABLE: &str = "LITELLM_LOG";
const JSON_LOGS_VARIABLE: &str = "JSON_LOGS";
const MAX_QUEUE_SIZE_VARIABLE: &str = "LOGGING_WORKER_MAX_QUEUE_SIZE";
const CONCURRENCY_VARIABLE: &str = "LOGGING_WORKER_CONCURRENCY";
const EVENT_TIMEOUT_VARIABLE: &str = "LOGGING_WORKER_MAX_TIME_PER_COROUTINE";

// LiteLLM's _logging module reads LITELLM_LOG with this default
pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Debug;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub service_callbacks: Vec<String>,
    pub log_level: LogLevel,
    pub json_logs: bool,
    pub max_queue_size: usize,
    pub concurrency: usize,
    pub event_timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            service_callbacks: Vec::new(),
            log_level: DEFAULT_LOG_LEVEL,
            json_logs: false,
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            event_timeout: DEFAULT_EVENT_TIMEOUT,
        }
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
        Ok(Settings {
            service_callbacks: resolve_names(
                self.litellm_settings.service_callback.unwrap_or_default(),
                lookup,
            )?,
            log_level: log_level(lookup)?,
            json_logs: json_logs(lookup),
            max_queue_size: variable(lookup, MAX_QUEUE_SIZE_VARIABLE)?
                .unwrap_or(DEFAULT_MAX_QUEUE_SIZE),
            concurrency: variable(lookup, CONCURRENCY_VARIABLE)?.unwrap_or(DEFAULT_CONCURRENCY),
            event_timeout: event_timeout(lookup)?,
        })
    }
}

fn resolve_names<L>(names: Vec<String>, lookup: &L) -> Result<Vec<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let path = format!("litellm_settings.service_callback.{index}");
            resolve_string(name, &path, lookup)
        })
        .collect()
}

// an empty variable counts as unset, matching how betterlite reads every environment value
fn log_level<L>(lookup: &L) -> Result<LogLevel, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(LOG_LEVEL_VARIABLE) else {
        return Ok(DEFAULT_LOG_LEVEL);
    };
    if text.is_empty() {
        return Ok(DEFAULT_LOG_LEVEL);
    }
    parse_level(&text).map_err(|_source| ConfigError::InvalidScalar {
        path: LOG_LEVEL_VARIABLE.to_owned(),
        value: false,
    })
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

fn variable<T, L>(lookup: &L, name: &str) -> Result<Option<T>, ConfigError>
where
    T: FromStr,
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(name) else {
        return Ok(None);
    };
    Ok(betterlite_env::number::<T>(name, &text)?)
}

fn event_timeout<L>(lookup: &L) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(seconds) = variable::<f64, _>(lookup, EVENT_TIMEOUT_VARIABLE)? else {
        return Ok(DEFAULT_EVENT_TIMEOUT);
    };
    Duration::try_from_secs_f64(seconds).map_err(|_source| ConfigError::InvalidScalar {
        path: EVENT_TIMEOUT_VARIABLE.to_owned(),
        value: false,
    })
}

#[path = "service_callback/tests.rs"]
mod tests;
