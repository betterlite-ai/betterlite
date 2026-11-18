use std::collections::BTreeSet;
use std::path::PathBuf;

use betterlite_db::database_diagnostics::level::{LevelError, LogLevel, parse_level};
use betterlite_env::{EnvError, flag, number};
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// LiteLLM declares every worker option as a click option, so the value arrives in the environment
const HOST_VARIABLE: &str = "HOST";
const PORT_VARIABLE: &str = "PORT";
const KEEPALIVE_VARIABLE: &str = "KEEPALIVE_TIMEOUT";
const HEALTHCHECK_VARIABLE: &str = "TIMEOUT_WORKER_HEALTHCHECK";
const MAX_REQUESTS_VARIABLE: &str = "MAX_REQUESTS_BEFORE_RESTART";
const JITTER_VARIABLE: &str = "MAX_REQUESTS_BEFORE_RESTART_JITTER";
const LOG_LEVEL_VARIABLE: &str = "LITELLM_LOG";
const JSON_LOGS_VARIABLE: &str = "JSON_LOGS";
const HOT_RELOAD_VARIABLE: &str = "LITELLM_DEV_ENV_HOT_RELOAD";

pub const DEFAULT_HOST: &str = "0.0.0.0";
pub const DEFAULT_PORT: u16 = 4000;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // the options come from flags and the variables LiteLLM's click declarations read, so this section owns no YAML key
    #[serde(skip)]
    _no_yaml_keys: (),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkerOptionsReloadSettings {
    pub worker_options: UvicornSettings,
    pub dev_reload: ReloadSettings,
}

// the settings are boxed so Sections stays one pointer wider, because their inline size pushed a db-admin future to clippy's 16384-byte large_futures limit
pub type Settings = Box<WorkerOptionsReloadSettings>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UvicornSettings {
    pub host: String,
    pub port: u16,
    pub server_header: bool,
    pub log_config: LogConfig,
    pub timeout_keep_alive: Option<i64>,
    pub timeout_worker_healthcheck: Option<i64>,
    pub limit_max_requests: Option<i64>,
    pub limit_max_requests_jitter: Option<i64>,
    pub loop_type: Option<LoopType>,
}

impl Default for UvicornSettings {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            server_header: false,
            log_config: LogConfig::Builtin,
            timeout_keep_alive: None,
            timeout_worker_healthcheck: None,
            limit_max_requests: None,
            limit_max_requests_jitter: None,
            loop_type: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LogConfig {
    #[default]
    Builtin,
    File(PathBuf),
    // LiteLLM's JSON config puts the resolved level on every uvicorn logger, DEBUG when LITELLM_LOG is unset
    Json {
        level: LogLevel,
    },
    Level(LogLevel),
    // the level is unusable, and the settings carry the typed error until the builder refuses to start
    Invalid(LogLevelError),
}

// the load keeps this error so the cause survives, and resolve_uvicorn_settings returns it to the caller
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LogLevelError {
    #[error("LITELLM_LOG value {value:?} is not a logging level name")]
    Unknown {
        value: String,
        #[source]
        source: LevelError,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReloadSettings {
    pub reload: bool,
    pub reload_dirs: Vec<PathBuf>,
    pub reload_includes: Vec<String>,
    // the StatReload patch watches these on top of *.py, resolved to absolute paths
    pub extra_paths: BTreeSet<PathBuf>,
    pub env_hot_reload: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopType {
    Uvloop,
}

// LiteLLM calls getattr(logging, level.upper()), which fails on a name logging does not define
pub fn resolve_log_level(level: &str) -> Result<LogLevel, LogLevelError> {
    parse_level(level).map_err(|source| LogLevelError::Unknown {
        value: level.to_owned(),
        source,
    })
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Box::new(WorkerOptionsReloadSettings {
            worker_options: UvicornSettings {
                host: variable(lookup, HOST_VARIABLE).unwrap_or_else(|| DEFAULT_HOST.to_owned()),
                port: parse(lookup, PORT_VARIABLE, number::<u16>)?.unwrap_or(DEFAULT_PORT),
                server_header: false,
                log_config: log_config(
                    variable(lookup, LOG_LEVEL_VARIABLE).as_deref(),
                    variable(lookup, JSON_LOGS_VARIABLE).as_deref(),
                ),
                timeout_keep_alive: parse(lookup, KEEPALIVE_VARIABLE, number::<i64>)?,
                timeout_worker_healthcheck: parse(lookup, HEALTHCHECK_VARIABLE, number::<i64>)?,
                limit_max_requests: parse(lookup, MAX_REQUESTS_VARIABLE, number::<i64>)?,
                limit_max_requests_jitter: parse(lookup, JITTER_VARIABLE, number::<i64>)?,
                loop_type: None,
            },
            dev_reload: ReloadSettings {
                reload: false,
                reload_dirs: Vec::new(),
                reload_includes: Vec::new(),
                extra_paths: BTreeSet::new(),
                env_hot_reload: parse(lookup, HOT_RELOAD_VARIABLE, flag)?.unwrap_or_default(),
            },
        }))
    }
}

// an empty variable counts as unset, as for every environment value betterlite reads
fn variable<L>(lookup: &L, name: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    match lookup.lookup(name) {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

// the parser is one of betterlite_env's readers, so the empty-value rule and the error shape stay shared
fn parse<L, T>(
    lookup: &L,
    name: &str,
    parser: fn(&str, &str) -> Result<Option<T>, EnvError>,
) -> Result<Option<T>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(value) = variable(lookup, name) else {
        return Ok(None);
    };
    Ok(parser(name, &value)?)
}

// LiteLLM's logging module resolves LITELLM_LOG even when JSON_LOGS gives uvicorn its JSON config
fn log_config(raw_level: Option<&str>, json_logs: Option<&str>) -> LogConfig {
    let json = json_logs.is_some_and(|value| value.to_lowercase() == "true");
    match raw_level.map(resolve_log_level) {
        Some(Err(error)) => LogConfig::Invalid(error),
        Some(Ok(level)) if json => LogConfig::Json { level },
        Some(Ok(level)) => LogConfig::Level(level),
        None if json => LogConfig::Json {
            level: LogLevel::Debug,
        },
        None => LogConfig::Builtin,
    }
}
