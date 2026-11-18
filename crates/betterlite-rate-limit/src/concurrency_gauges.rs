pub mod batch;
pub mod cache;
pub mod lua_scripts;

mod acquire_in_memory;
mod batch_get;
mod batch_read;
mod cache_value_conversion;
mod check_acquire;
mod check_read_only;
mod defer_release;
mod mirror_count;
mod release_in_memory;
mod release_redis;
mod release_stash;
mod status_builder;
mod tests;

use std::sync::Arc;
use std::time::{Duration, SystemTimeError, UNIX_EPOCH};

use tokio::sync::Mutex;

use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::limiter_records::{
    CacheCounterValue, CounterReplyError, LimiterClock, ParallelRequestGauge, RateLimitResponse,
};
use crate::rate_limit_window_store::CounterScript;
use crate::request_stash::RequestStash;

use batch::ReleaseBatchSource;
use cache::GaugeCache;

// LiteLLM enforces max_parallel_requests as a slot registry whose PARALLEL_REQUEST_SLOT_TTL_SECONDS is 3600
pub const PARALLEL_SLOT_TTL: Duration = Duration::from_secs(3600);

pub struct ConcurrencyGauges {
    pub cache: Arc<dyn GaugeCache>,
    pub clock: Arc<dyn LimiterClock>,
    pub acquire_script: Option<Arc<dyn CounterScript>>,
    pub release_script: Option<Arc<dyn CounterScript>>,
    pub count_script: Option<Arc<dyn CounterScript>>,
    pub release_batch: Option<Arc<dyn ReleaseBatchSource>>,
    pub fail_closed: FailClosedPolicy,
    // LiteLLM's _check_and_increment_lock, held across the reads and writes of one local gauge pass
    pub check_and_increment_lock: Mutex<()>,
}

impl ConcurrencyGauges {
    pub async fn check_parallel_request_gauges(
        &self,
        gauges: &[ParallelRequestGauge],
        slot_id: &str,
        read_only: bool,
    ) -> Result<RateLimitResponse, GaugeError> {
        let gauge_keys: Vec<String> = gauges
            .iter()
            .map(|gauge| gauge.counter_key.clone())
            .collect();
        if read_only {
            return check_read_only::check_read_only(self, gauges, &gauge_keys).await;
        }
        check_acquire::check_acquire(self, gauges, &gauge_keys, slot_id).await
    }

    pub async fn release_stashed_parallel_slot(
        &self,
        stash: Option<&RequestStash>,
        in_logging_callback: bool,
    ) {
        if let Some(stash) = stash {
            release_stash::release_stashed_slot(self, stash, in_logging_callback).await;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GaugeTimes {
    pub(crate) now: f64,
    pub(crate) cutoff: f64,
}

pub(crate) fn gauge_times(clock: &dyn LimiterClock) -> Result<GaugeTimes, GaugeError> {
    let elapsed = clock
        .current_time()
        .duration_since(UNIX_EPOCH)
        .map_err(GaugeError::ClockBeforeEpoch)?;
    let cutoff = match elapsed.checked_sub(PARALLEL_SLOT_TTL) {
        Some(older) => older.as_secs_f64(),
        None => f64::NEG_INFINITY,
    };
    Ok(GaugeTimes {
        now: elapsed.as_secs_f64(),
        cutoff,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum GaugeError {
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
    #[error(transparent)]
    Reply(#[from] CounterReplyError),
    #[error("the gauge value {value:?} is not a whole number")]
    NotWholeNumber { value: CacheCounterValue },
    #[error("the parallel acquire script returned no element")]
    MissingAcquireReply,
    #[error("the parallel acquire script reply holds {length} elements, but {needed} are needed")]
    ShortAcquireReply { needed: usize, length: usize },
    #[error("the parallel acquire script names gauge {index}, but the request carries {gauges}")]
    GaugeIndex { index: i64, gauges: usize },
    #[error("the limiter clock reads a time before the Unix epoch")]
    ClockBeforeEpoch(#[source] SystemTimeError),
    #[error(transparent)]
    Redis(#[from] betterlite_redis::redis_counters::RedisCountersError),
}
