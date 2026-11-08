use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

pub const DATABASE_VARIABLE: &str = "CLICKHOUSE_DATABASE";
pub const URL_VARIABLE: &str = "CLICKHOUSE_URL";
pub const BATCH_SIZE_VARIABLE: &str = "CLICKHOUSE_BATCH_SIZE";
pub const FLUSH_INTERVAL_VARIABLE: &str = "CLICKHOUSE_FLUSH_INTERVAL_SECONDS";
pub const MAX_BUFFERED_ROWS_VARIABLE: &str = "CLICKHOUSE_MAX_BUFFERED_ROWS";
pub const MAX_RETRIES_VARIABLE: &str = "CLICKHOUSE_MAX_RETRIES";
pub const DEFAULT_DATABASE: &str = "litellm";
pub const DEFAULT_BATCH_SIZE: usize = 10_000;
pub const DEFAULT_FLUSH_INTERVAL_SECONDS: f64 = 1.0;
pub const DEFAULT_MAX_BUFFERED_ROWS: usize = 200_000;
pub const DEFAULT_MAX_RETRIES: usize = 3;

#[derive(Clone, Debug)]
pub struct ClickHouseSettings {
    pub database: String,
    pub url: SecretString,
    pub batch_size: usize,
    pub flush_interval: Duration,
    pub max_buffered_rows: usize,
    pub max_retries: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ClickHouseSettingsError {
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
    #[error("environment variable {name} is not a duration in seconds")]
    Interval {
        name: &'static str,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("environment variable {name} must be greater than zero")]
    Zero { name: &'static str },
}

fn configured_count(
    environment: &dyn EnvironmentLookup,
    name: &'static str,
    default: usize,
) -> Result<usize, ClickHouseSettingsError> {
    let configured = environment
        .lookup(name)
        .map(|text| betterlite_env::number::<usize>(name, &text))
        .transpose()?
        .flatten()
        .unwrap_or(default);
    Ok(configured)
}

fn configured_interval(
    environment: &dyn EnvironmentLookup,
) -> Result<Duration, ClickHouseSettingsError> {
    let seconds = environment
        .lookup(FLUSH_INTERVAL_VARIABLE)
        .map(|text| betterlite_env::number::<f64>(FLUSH_INTERVAL_VARIABLE, &text))
        .transpose()?
        .flatten()
        .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS);
    Duration::try_from_secs_f64(seconds).map_err(|source| ClickHouseSettingsError::Interval {
        name: FLUSH_INTERVAL_VARIABLE,
        source,
    })
}

impl ClickHouseSettings {
    pub fn from_environment(
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, ClickHouseSettingsError> {
        let database = environment
            .lookup(DATABASE_VARIABLE)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_DATABASE.to_owned());
        let url = environment.lookup(URL_VARIABLE).unwrap_or_default();
        let batch_size = configured_count(environment, BATCH_SIZE_VARIABLE, DEFAULT_BATCH_SIZE)?;
        if batch_size == 0 {
            return Err(ClickHouseSettingsError::Zero {
                name: BATCH_SIZE_VARIABLE,
            });
        }
        let flush_interval = configured_interval(environment)?;
        let max_buffered_rows = configured_count(
            environment,
            MAX_BUFFERED_ROWS_VARIABLE,
            DEFAULT_MAX_BUFFERED_ROWS,
        )?;
        let max_retries = configured_count(environment, MAX_RETRIES_VARIABLE, DEFAULT_MAX_RETRIES)?;
        Ok(Self {
            database,
            url: SecretString::from(url),
            batch_size,
            flush_interval,
            max_buffered_rows,
            max_retries,
        })
    }
}

mod tests;
