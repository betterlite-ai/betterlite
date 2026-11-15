use std::collections::VecDeque;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_env::{EnvError, flag, number};
use thiserror::Error;

use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::{GcsBucketLogger, GcsDeps};

pub const BATCH_SIZE_ENV: &str = "GCS_BATCH_SIZE";
pub const FLUSH_INTERVAL_ENV: &str = "GCS_FLUSH_INTERVAL";
pub const USE_BATCHED_LOGGING_ENV: &str = "GCS_USE_BATCHED_LOGGING";
pub const QUEUE_SIZE_ENV: &str = "LITELLM_ASYNCIO_QUEUE_MAXSIZE";
pub const DEFAULT_BATCH_SIZE: usize = 2048;
pub const DEFAULT_FLUSH_INTERVAL_SECONDS: u64 = 20;
pub const DEFAULT_USE_BATCHED_LOGGING: bool = true;
pub const DEFAULT_QUEUE_SIZE: usize = 1000;
// CustomBatchLogger.__init__ replaces a falsy batch size and flush interval with litellm.DEFAULT_BATCH_SIZE and DEFAULT_FLUSH_INTERVAL_SECONDS
pub const BASE_BATCH_SIZE: usize = 512;
pub const BASE_FLUSH_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("the GCS logging setting {name} is invalid")]
    Environment {
        name: &'static str,
        #[source]
        source: EnvError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcsSettings {
    pub batch_size: usize,
    pub flush_interval: Duration,
    pub use_batched_logging: bool,
    pub queue_capacity: usize,
}

impl GcsSettings {
    pub fn from_environment(environment: &dyn EnvironmentSource) -> Result<Self, SettingsError> {
        let batch_size = match setting::<usize>(environment, BATCH_SIZE_ENV)? {
            None => DEFAULT_BATCH_SIZE,
            Some(0) => BASE_BATCH_SIZE,
            Some(size) => size,
        };
        let flush_seconds = match setting::<u64>(environment, FLUSH_INTERVAL_ENV)? {
            None => DEFAULT_FLUSH_INTERVAL_SECONDS,
            Some(0) => BASE_FLUSH_INTERVAL_SECONDS,
            Some(seconds) => seconds,
        };
        let queue_capacity =
            setting::<usize>(environment, QUEUE_SIZE_ENV)?.unwrap_or(DEFAULT_QUEUE_SIZE);
        let use_batched_logging = match environment.get(USE_BATCHED_LOGGING_ENV) {
            None => DEFAULT_USE_BATCHED_LOGGING,
            Some(text) => flag(USE_BATCHED_LOGGING_ENV, &text)
                .map_err(|source| SettingsError::Environment {
                    name: USE_BATCHED_LOGGING_ENV,
                    source,
                })?
                .unwrap_or(DEFAULT_USE_BATCHED_LOGGING),
        };
        Ok(Self {
            batch_size,
            flush_interval: Duration::from_secs(flush_seconds),
            use_batched_logging,
            queue_capacity,
        })
    }
}

fn setting<T>(
    environment: &dyn EnvironmentSource,
    name: &'static str,
) -> Result<Option<T>, SettingsError>
where
    T: FromStr,
{
    let Some(text) = environment.get(name) else {
        return Ok(None);
    };
    number::<T>(name, &text).map_err(|source| SettingsError::Environment { name, source })
}

// LiteLLM reads the settings in __init__, then starts periodic_flush with asyncio.create_task
pub fn start<Auth, Http>(
    base: GcsBucketBase<Auth, Http>,
    deps: GcsDeps,
) -> Result<Arc<GcsBucketLogger<Auth, Http>>, SettingsError>
where
    Auth: VertexAuth + Clone + Debug + 'static,
    Http: GcsHttpClient + Debug + 'static,
{
    let settings = GcsSettings::from_environment(base.environment.as_ref())?;
    let last_flush_time = deps.clock.now();
    let logger = Arc::new(GcsBucketLogger {
        base,
        settings,
        queue: Mutex::new(VecDeque::new()),
        last_flush_time: Mutex::new(last_flush_time),
        clock: deps.clock,
        ids: deps.ids,
        stopped: deps.shutdown.clone(),
    });
    if let Some(tasks) = deps.tasks {
        let periodic = Arc::clone(&logger);
        tasks.spawn(async move { periodic.periodic_flush().await });
    }
    Ok(logger)
}

mod tests;
