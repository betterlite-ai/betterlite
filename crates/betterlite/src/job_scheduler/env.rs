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
        // LiteLLM spells this one out with .lower() in ["true", "1"]: every other value is false, never an error.
        let coalesce = env.get("APSCHEDULER_COALESCE").is_none_or(|text| {
            let lowered = text.to_lowercase();
            lowered == "true" || lowered == "1"
        });
        let misfire_grace_seconds =
            env_number::<u64>(env, "APSCHEDULER_MISFIRE_GRACE_TIME")?.unwrap_or(3600);
        let max_instances = env_number::<u32>(env, "APSCHEDULER_MAX_INSTANCES")?.unwrap_or(1);
        // APScheduler's Job rejects both zero values with a TypeError, so LiteLLM stops at startup.
        if misfire_grace_seconds == 0 {
            return Err(SchedulerError::InvalidEnv {
                name: "APSCHEDULER_MISFIRE_GRACE_TIME".to_owned(),
            });
        }
        if max_instances == 0 {
            return Err(SchedulerError::InvalidEnv {
                name: "APSCHEDULER_MAX_INSTANCES".to_owned(),
            });
        }
        Ok(Self {
            coalesce,
            misfire_grace_seconds,
            max_instances,
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
            finish: env_duration(env, "SCHEDULED_JOB_SHUTDOWN_FINISH_TIMEOUT_SECONDS", 5.0)?,
            cancel: env_duration(env, "SCHEDULED_JOB_SHUTDOWN_CANCEL_TIMEOUT_SECONDS", 5.0)?,
        })
    }
}

fn env_number<T>(env: &BTreeMap<String, String>, name: &str) -> Result<Option<T>, SchedulerError>
where
    T: FromStr,
{
    let Some(text) = env.get(name) else {
        return Ok(None);
    };
    betterlite_env::number::<T>(name, text).map_err(|_source| SchedulerError::InvalidEnv {
        name: name.to_owned(),
    })
}

fn env_duration(
    env: &BTreeMap<String, String>,
    name: &str,
    default: f64,
) -> Result<Duration, SchedulerError> {
    let seconds = env_number::<f64>(env, name)?.unwrap_or(default);
    // LiteLLM passes a negative float to asyncio.wait, which returns at once; betterlite treats it as zero.
    if seconds.is_sign_negative() {
        return Ok(Duration::ZERO);
    }
    Duration::try_from_secs_f64(seconds).map_err(|_source| SchedulerError::InvalidEnv {
        name: name.to_owned(),
    })
}

mod tests;
