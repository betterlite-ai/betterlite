use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

use super::SchedulerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobDefaults {
    pub coalesce: bool,
    pub misfire_grace_seconds: u64,
    pub max_instances: u32,
}

impl JobDefaults {
    pub fn from_env(env: &BTreeMap<String, String>) -> Result<Self, SchedulerError> {
        Ok(Self {
            coalesce: env_flag(env, "APSCHEDULER_COALESCE", true)?,
            misfire_grace_seconds: env_number(env, "APSCHEDULER_MISFIRE_GRACE_TIME", 3600_u64)?,
            max_instances: env_number(env, "APSCHEDULER_MAX_INSTANCES", 1_u32)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownTimeouts {
    pub finish: Duration,
    pub cancel: Duration,
}

impl ShutdownTimeouts {
    pub fn from_env(env: &BTreeMap<String, String>) -> Result<Self, SchedulerError> {
        Ok(Self {
            finish: env_seconds(env, "SCHEDULED_JOB_SHUTDOWN_FINISH_TIMEOUT_SECONDS", 5.0)?,
            cancel: env_seconds(env, "SCHEDULED_JOB_SHUTDOWN_CANCEL_TIMEOUT_SECONDS", 5.0)?,
        })
    }
}

fn env_flag(
    env: &BTreeMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool, SchedulerError> {
    let Some(text) = env.get(name) else {
        return Ok(default);
    };
    let value = betterlite_env::flag(name, text).map_err(|_source| SchedulerError::InvalidEnv {
        name: name.to_owned(),
    })?;
    Ok(value.unwrap_or(default))
}

fn env_number<T>(
    env: &BTreeMap<String, String>,
    name: &str,
    default: T,
) -> Result<T, SchedulerError>
where
    T: FromStr,
{
    let Some(text) = env.get(name) else {
        return Ok(default);
    };
    let value =
        betterlite_env::number::<T>(name, text).map_err(|_source| SchedulerError::InvalidEnv {
            name: name.to_owned(),
        })?;
    Ok(value.unwrap_or(default))
}

fn env_seconds(
    env: &BTreeMap<String, String>,
    name: &str,
    default: f64,
) -> Result<Duration, SchedulerError> {
    let seconds = env_number::<f64>(env, name, default)?;
    Duration::try_from_secs_f64(seconds).map_err(|_source| SchedulerError::InvalidEnv {
        name: name.to_owned(),
    })
}

mod tests;
