use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use betterlite_db::database_diagnostics::level::{LogLevel, parse_level};
use serde::Deserialize;

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
pub struct Settings {
    pub worker_options: UvicornSettings,
    pub dev_reload: ReloadSettings,
}

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
    Json,
    Level(LogLevel),
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

impl fmt::Display for LoopType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uvloop => formatter.write_str("uvloop"),
        }
    }
}

// LiteLLM calls getattr(logging, level.upper()), which fails on a name logging does not define
pub fn resolve_log_level(level: &str) -> Result<LogLevel, ConfigError> {
    parse_level(level).map_err(|_source| ConfigError::InvalidScalar {
        path: LOG_LEVEL_VARIABLE.to_owned(),
        value: false,
    })
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            worker_options: UvicornSettings {
                host: variable(lookup, HOST_VARIABLE).unwrap_or_else(|| DEFAULT_HOST.to_owned()),
                port: port(lookup)?,
                server_header: false,
                log_config: log_config(lookup)?,
                timeout_keep_alive: seconds(lookup, KEEPALIVE_VARIABLE)?,
                timeout_worker_healthcheck: seconds(lookup, HEALTHCHECK_VARIABLE)?,
                limit_max_requests: seconds(lookup, MAX_REQUESTS_VARIABLE)?,
                limit_max_requests_jitter: seconds(lookup, JITTER_VARIABLE)?,
                loop_type: None,
            },
            dev_reload: ReloadSettings {
                reload: false,
                reload_dirs: Vec::new(),
                reload_includes: Vec::new(),
                extra_paths: BTreeSet::new(),
                env_hot_reload: flag(lookup, HOT_RELOAD_VARIABLE)?,
            },
        })
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

fn seconds<L>(lookup: &L, name: &str) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    match variable(lookup, name) {
        Some(value) => Ok(betterlite_env::number::<i64>(name, &value)?),
        None => Ok(None),
    }
}

fn flag<L>(lookup: &L, name: &str) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match variable(lookup, name) {
        Some(value) => Ok(betterlite_env::flag(name, &value)?.unwrap_or_default()),
        None => Ok(false),
    }
}

fn port<L>(lookup: &L) -> Result<u16, ConfigError>
where
    L: EnvironmentLookup,
{
    match variable(lookup, PORT_VARIABLE) {
        Some(value) => {
            let parsed = betterlite_env::number::<u16>(PORT_VARIABLE, &value)?;
            Ok(parsed.unwrap_or(DEFAULT_PORT))
        }
        None => Ok(DEFAULT_PORT),
    }
}

fn json_logs<L>(lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    match variable(lookup, JSON_LOGS_VARIABLE) {
        Some(value) => value.to_lowercase() == "true",
        None => false,
    }
}

fn log_config<L>(lookup: &L) -> Result<LogConfig, ConfigError>
where
    L: EnvironmentLookup,
{
    // LiteLLM's logging module resolves LITELLM_LOG even when JSON_LOGS gives uvicorn its JSON config
    let level = variable(lookup, LOG_LEVEL_VARIABLE)
        .as_deref()
        .map(resolve_log_level)
        .transpose()?;
    if json_logs(lookup) {
        return Ok(LogConfig::Json);
    }
    Ok(level.map_or(LogConfig::Builtin, LogConfig::Level))
}
