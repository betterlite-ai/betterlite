use std::time::Duration;

use betterlite_db::spend_flush::monitor::{
    DEFAULT_POLL_INTERVAL, DEFAULT_QUEUE_SIZE_THRESHOLD, SpendMonitorSettings,
};
use betterlite_env::EnvError;

use super::initial_environment_integer;
use crate::loader::{ConfigError, EnvironmentLookup};

pub const ENV_QUEUE_SIZE_THRESHOLD: &str = "SPEND_LOG_QUEUE_SIZE_THRESHOLD";
pub const ENV_QUEUE_POLL_INTERVAL: &str = "SPEND_LOG_QUEUE_POLL_INTERVAL";

// SPEND_LOG_QUEUE_SIZE_THRESHOLD and SPEND_LOG_QUEUE_POLL_INTERVAL of litellm/constants.py, read once here so no pass consults the environment again.
pub(super) fn monitor_settings<L>(lookup: &L) -> Result<SpendMonitorSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(SpendMonitorSettings {
        queue_size_threshold: threshold(lookup)?,
        base_interval: poll_interval(lookup)?,
    })
}

// int(os.getenv(...)): any integer loads, and the threshold only decides the backoff, so a nonpositive one is not clamped.
fn threshold<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    initial_environment_integer(
        lookup,
        ENV_QUEUE_SIZE_THRESHOLD,
        DEFAULT_QUEUE_SIZE_THRESHOLD,
    )
}

// float(os.getenv(...)): a finite nonpositive value polls at once, and a value with no representable timer is refused under the setting's name.
fn poll_interval<L>(lookup: &L) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ENV_QUEUE_POLL_INTERVAL) else {
        return Ok(DEFAULT_POLL_INTERVAL);
    };
    let parsed = betterlite_env::number::<f64>(ENV_QUEUE_POLL_INTERVAL, &text)
        .map_err(|source| ConfigError::Environment { source })?;
    let Some(seconds) = parsed else {
        return Ok(DEFAULT_POLL_INTERVAL);
    };
    if !seconds.is_finite() {
        return Err(setting_error());
    }
    if seconds <= 0.0 {
        return Ok(Duration::ZERO);
    }
    match Duration::try_from_secs_f64(seconds) {
        Ok(interval) => Ok(interval),
        // a finite interval past the Duration range is as unschedulable as an infinite one
        Err(_overflow) => Err(setting_error()),
    }
}

fn setting_error() -> ConfigError {
    ConfigError::Environment {
        source: EnvError::Number {
            name: ENV_QUEUE_POLL_INTERVAL.to_owned(),
        },
    }
}
