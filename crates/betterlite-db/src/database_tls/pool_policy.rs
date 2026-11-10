use betterlite_env::EnvError;

use crate::database_urls::environment::EnvironmentSource;

pub const ENABLED_ENV: &str = "LITELLM_PGBOUNCER_ENABLED";
pub const PORT_ENV: &str = "LITELLM_PGBOUNCER_PORT";
pub const MAX_DB_CONNECTIONS_ENV: &str = "LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS";
pub const MAX_CLIENT_CONN_ENV: &str = "LITELLM_PGBOUNCER_MAX_CLIENT_CONN";
pub const BINARY_ENV: &str = "LITELLM_PGBOUNCER_BINARY";

pub const DEFAULT_PORT: u16 = 6432;
pub const DEFAULT_MAX_DB_CONNECTIONS: i64 = 20;
pub const DEFAULT_MAX_CLIENT_CONN: i64 = 1000;
pub const DEFAULT_BINARY: &str = "pgbouncer";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgBouncerSettings {
    pub enabled: bool,
    pub port: u16,
    pub max_db_connections: i64,
    pub max_client_conn: i64,
    pub binary: String,
}

impl Default for PgBouncerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            max_db_connections: DEFAULT_MAX_DB_CONNECTIONS,
            max_client_conn: DEFAULT_MAX_CLIENT_CONN,
            binary: DEFAULT_BINARY.to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct PoolEnvironment {
    pub settings: PgBouncerSettings,
    // settings read but not honored: the native gateway never launches an embedded PgBouncer
    pub ignored: Vec<&'static str>,
}

#[derive(Debug, thiserror::Error)]
pub enum PoolSettingsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("environment variable {name} must be at least {minimum}, got {value}")]
    TooSmall {
        name: String,
        value: i64,
        minimum: i64,
    },
    #[error("environment variable {name} must be at most {maximum}, got {value}")]
    TooLarge {
        name: String,
        value: i64,
        maximum: i64,
    },
}

// Mirrors pgbouncer.py's PgBouncerSettings: env-only names, defaults and Field bounds, extra names ignored.
pub fn from_env<S>(environment: &S) -> Result<PoolEnvironment, PoolSettingsError>
where
    S: EnvironmentSource,
{
    let enabled = boolean(environment.lookup(ENABLED_ENV).as_deref(), ENABLED_ENV)?;
    let configured_port = port(environment.lookup(PORT_ENV).as_deref())?;
    let max_db_connections = count(
        environment.lookup(MAX_DB_CONNECTIONS_ENV).as_deref(),
        MAX_DB_CONNECTIONS_ENV,
        DEFAULT_MAX_DB_CONNECTIONS,
    )?;
    let max_client_conn = count(
        environment.lookup(MAX_CLIENT_CONN_ENV).as_deref(),
        MAX_CLIENT_CONN_ENV,
        DEFAULT_MAX_CLIENT_CONN,
    )?;
    let binary = environment
        .lookup(BINARY_ENV)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BINARY.to_owned());
    let ignored = [PORT_ENV, MAX_CLIENT_CONN_ENV, BINARY_ENV]
        .into_iter()
        .filter(|name| {
            environment
                .lookup(name)
                .is_some_and(|text| !text.trim().is_empty())
        })
        .collect();
    Ok(PoolEnvironment {
        settings: PgBouncerSettings {
            enabled,
            port: configured_port,
            max_db_connections,
            max_client_conn,
            binary,
        },
        ignored,
    })
}

fn boolean(configured: Option<&str>, name: &str) -> Result<bool, PoolSettingsError> {
    let Some(text) = configured else {
        return Ok(false);
    };
    Ok(betterlite_env::flag(name, text)?.unwrap_or(false))
}

fn integer(configured: Option<&str>, name: &str) -> Result<Option<i64>, PoolSettingsError> {
    let Some(text) = configured else {
        return Ok(None);
    };
    betterlite_env::number(name, text).map_err(PoolSettingsError::from)
}

fn at_least(name: &str, value: i64, minimum: i64) -> Result<i64, PoolSettingsError> {
    if value < minimum {
        return Err(PoolSettingsError::TooSmall {
            name: name.to_owned(),
            value,
            minimum,
        });
    }
    Ok(value)
}

fn port(text: Option<&str>) -> Result<u16, PoolSettingsError> {
    let Some(value) = integer(text, PORT_ENV)? else {
        return Ok(DEFAULT_PORT);
    };
    let checked = at_least(PORT_ENV, value, 1)?;
    u16::try_from(checked).map_err(|_source| PoolSettingsError::TooLarge {
        name: PORT_ENV.to_owned(),
        value: checked,
        maximum: i64::from(u16::MAX),
    })
}

fn count(text: Option<&str>, name: &str, default: i64) -> Result<i64, PoolSettingsError> {
    let Some(value) = integer(text, name)? else {
        return Ok(default);
    };
    at_least(name, value, 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Writer,
    Reader,
    Migration,
}

// LiteLLM's embedded PgBouncer caps a pod's upstream connections; only the writer pool stands in for it.
#[must_use]
pub fn pool_limit(role: SessionRole, configured: i64, settings: &PgBouncerSettings) -> i64 {
    match role {
        SessionRole::Writer if settings.enabled => configured.min(settings.max_db_connections),
        SessionRole::Writer | SessionRole::Reader | SessionRole::Migration => configured,
    }
}
