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
    let capacity = read_setting(lookup, QUEUE_CAPACITY_ENV)?.unwrap_or(DEFAULT_QUEUE_CAPACITY);
    let aggregation_threshold = match read_setting(lookup, AGGREGATION_THRESHOLD_ENV)? {
        Some(threshold) => threshold,
        None => scaled_threshold(capacity)?,
    };
    Ok(SpendQueueSettings {
        queue_capacity: capacity,
        aggregation_threshold,
        flush_count: read_setting(lookup, FLUSH_COUNT_ENV)?.unwrap_or(DEFAULT_FLUSH_COUNT),
        default_lock_ttl_seconds: read_setting(lookup, DEFAULT_LOCK_TTL_ENV)?
            .unwrap_or(DEFAULT_LOCK_TTL_SECONDS),
    })
}

// An absent variable reads as the empty string, which the decoder treats as unset.
fn read_setting<L>(lookup: &L, name: &str) -> Result<Option<i64>, SpendQueueError>
where
    L: EnvironmentLookup,
{
    let text = lookup.lookup(name).unwrap_or_default();
    decode_spend_queue_setting(name, &text)
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
