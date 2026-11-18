use crate::loader::EnvironmentLookup;
use betterlite_env::EnvError;

mod tests;

pub const QUEUE_CAPACITY_ENV: &str = "LITELLM_ASYNCIO_QUEUE_MAXSIZE";
pub const AGGREGATION_THRESHOLD_ENV: &str = "MAX_SIZE_IN_MEMORY_QUEUE";
pub const FLUSH_COUNT_ENV: &str = "MAX_IN_MEMORY_QUEUE_FLUSH_COUNT";
pub const DEFAULT_LOCK_TTL_ENV: &str = "DEFAULT_CRON_JOB_LOCK_TTL_SECONDS";
pub const DEFAULT_QUEUE_CAPACITY: i64 = 1000;
pub const DEFAULT_FLUSH_COUNT: i64 = 1000;
pub const DEFAULT_LOCK_TTL_SECONDS: i64 = 60;

const THRESHOLD_PERCENT: i64 = 4;
const PERCENT_SCALE: i64 = 5;

// The in-memory spend queue and cron lease constants of litellm/constants.py, read from the loader's environment once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendQueueSettings {
    pub queue_capacity: i64,
    pub aggregation_threshold: i64,
    pub flush_count: i64,
    pub default_lock_ttl_seconds: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendQueueError {
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("environment variable {name} must be a positive integer")]
    NotPositive { name: String },
    #[error("environment variable {name} is too large to derive the spend queue threshold")]
    Overflow { name: String },
}

// The source derives MAX_SIZE_IN_MEMORY_QUEUE as int(capacity * 0.8), which is floor(capacity * 4 / 5).
pub fn decode_spend_queue_setting(name: &str, text: &str) -> Result<Option<i64>, SpendQueueError> {
    let value = betterlite_env::number::<i64>(name, text)?;
    if matches!(name, QUEUE_CAPACITY_ENV | DEFAULT_LOCK_TTL_ENV)
        && value.is_some_and(|parsed| parsed <= 0)
    {
        // asyncio.Queue(maxsize=0) is unbounded and Redis SET EX 0 fails, so betterlite refuses both instead of starting degraded.
        return Err(SpendQueueError::NotPositive {
            name: name.to_owned(),
        });
    }
    Ok(value)
}

pub fn load_spend_queue_settings<L>(lookup: &L) -> Result<SpendQueueSettings, SpendQueueError>
where
    L: EnvironmentLookup,
{
    // An absent variable reads as the empty string, which the decoder treats as unset.
    let capacity_text = lookup.lookup(QUEUE_CAPACITY_ENV).unwrap_or_default();
    let capacity = decode_spend_queue_setting(QUEUE_CAPACITY_ENV, &capacity_text)?
        .unwrap_or(DEFAULT_QUEUE_CAPACITY);
    let threshold_text = lookup.lookup(AGGREGATION_THRESHOLD_ENV).unwrap_or_default();
    let aggregation_threshold =
        match decode_spend_queue_setting(AGGREGATION_THRESHOLD_ENV, &threshold_text)? {
            Some(threshold) => threshold,
            None => scaled_threshold(capacity)?,
        };
    let flush_text = lookup.lookup(FLUSH_COUNT_ENV).unwrap_or_default();
    let flush_count =
        decode_spend_queue_setting(FLUSH_COUNT_ENV, &flush_text)?.unwrap_or(DEFAULT_FLUSH_COUNT);
    let ttl_text = lookup.lookup(DEFAULT_LOCK_TTL_ENV).unwrap_or_default();
    let default_lock_ttl_seconds = decode_spend_queue_setting(DEFAULT_LOCK_TTL_ENV, &ttl_text)?
        .unwrap_or(DEFAULT_LOCK_TTL_SECONDS);
    Ok(SpendQueueSettings {
        queue_capacity: capacity,
        aggregation_threshold,
        flush_count,
        default_lock_ttl_seconds,
    })
}

fn scaled_threshold(capacity: i64) -> Result<i64, SpendQueueError> {
    let overflow = SpendQueueError::Overflow {
        name: QUEUE_CAPACITY_ENV.to_owned(),
    };
    let scaled = capacity.checked_mul(THRESHOLD_PERCENT).ok_or(overflow)?;
    scaled
        .checked_div(PERCENT_SCALE)
        .ok_or(SpendQueueError::Overflow {
            name: QUEUE_CAPACITY_ENV.to_owned(),
        })
}
