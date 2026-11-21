use std::time::Duration;

use betterlite_db::spend_retention::batches::settings::CleanupBounds;
use betterlite_env::EnvError;

use crate::loader::{ConfigError, EnvironmentLookup};

pub const ENV_BATCH_SIZE: &str = "SPEND_LOG_CLEANUP_BATCH_SIZE";
pub const ENV_RUN_LOOPS: &str = "SPEND_LOG_RUN_LOOPS";
pub const ENV_RUN_BUDGET: &str = "SPEND_LOG_CLEANUP_RUN_BUDGET_SECONDS";
pub const ENV_BATCH_TIMEOUT: &str = "SPEND_LOG_CLEANUP_BATCH_TIMEOUT_SECONDS";
pub const ENV_MAX_FAILURES: &str = "SPEND_LOG_CLEANUP_MAX_CONSECUTIVE_BATCH_FAILURES";
pub const ENV_FAILURE_BACKOFF: &str = "SPEND_LOG_CLEANUP_BATCH_FAILURE_BACKOFF_SECONDS";
pub const ENV_COUNT_CAP: &str = "SPEND_LOG_CLEANUP_REMAINING_COUNT_CAP";

// litellm/constants.py reads these once at import, so a malformed number stops the proxy from starting
pub fn cleanup_defaults<Lookup: EnvironmentLookup>(
    lookup: &Lookup,
) -> Result<CleanupBounds, ConfigError> {
    Ok(CleanupBounds {
        batch_size: betterlite_env::number(
            ENV_BATCH_SIZE,
            &lookup.lookup(ENV_BATCH_SIZE).unwrap_or_default(),
        )?
        .unwrap_or(1_000),
        max_batches: betterlite_env::number(
            ENV_RUN_LOOPS,
            &lookup.lookup(ENV_RUN_LOOPS).unwrap_or_default(),
        )?
        .unwrap_or(500),
        run_budget: seconds(lookup, ENV_RUN_BUDGET)?.unwrap_or(Duration::from_secs(300)),
        batch_timeout: seconds(lookup, ENV_BATCH_TIMEOUT)?.unwrap_or(Duration::from_secs(30)),
        max_consecutive_failures: betterlite_env::number(
            ENV_MAX_FAILURES,
            &lookup.lookup(ENV_MAX_FAILURES).unwrap_or_default(),
        )?
        .unwrap_or(3),
        failure_backoff: seconds(lookup, ENV_FAILURE_BACKOFF)?
            .unwrap_or(Duration::from_millis(500)),
        remaining_count_cap: betterlite_env::number(
            ENV_COUNT_CAP,
            &lookup.lookup(ENV_COUNT_CAP).unwrap_or_default(),
        )?
        .unwrap_or(100_000),
    })
}

// asyncio waits not at all for a negative number of seconds, which a zero Duration matches
fn seconds<Lookup: EnvironmentLookup>(
    lookup: &Lookup,
    name: &str,
) -> Result<Option<Duration>, ConfigError> {
    let text = lookup.lookup(name).unwrap_or_default();
    let Some(parsed) = betterlite_env::number::<f64>(name, &text)? else {
        return Ok(None);
    };
    let overflow = EnvError::Number {
        name: name.to_owned(),
    };
    let duration = Duration::try_from_secs_f64(parsed.max(0.0_f64)).map_err(|_error| overflow)?;
    Ok(Some(duration))
}
