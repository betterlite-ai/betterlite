use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_cache::backend::Ttl;
use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::MemoryMaxSize as _;
use betterlite_context::local_ttl_cache::CacheValue;

use super::error::LoginThrottleError;

// InMemoryCache's max_size_in_memory for the two per-worker stores
pub const MAX_TRACKED_COUNTERS: usize = 20_000;
pub const MAX_TRACKED_BLOCKS: usize = 10_000;
const MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

// the four InMemoryCache methods login_throttle.py calls, returning typed failures instead of raising
pub trait LocalStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<CacheValue>, LoginThrottleError>;
    fn set(&self, key: &str, value: CacheValue, seconds: i64) -> Result<(), LoginThrottleError>;
    fn increment(&self, key: &str, seconds: i64) -> Result<f64, LoginThrottleError>;
    fn delete(&self, key: &str) -> Result<(), LoginThrottleError>;
}

#[derive(Clone)]
pub struct ThrottleStores {
    pub counters: Arc<dyn LocalStore>,
    pub blocks: Arc<dyn LocalStore>,
}

// B125's bounded local cache with LiteLLM's two capacities
pub fn local_stores_with_limits(
    clock: Clock,
    counters: usize,
    blocks: usize,
) -> Result<ThrottleStores, LoginThrottleError> {
    let counters_store = Arc::new(
        LocalMemoryOps::new(MAX_SIZE_PER_ITEM_KIB, Arc::clone(&clock))
            .map_err(LoginThrottleError::from)?,
    );
    counters_store.set_max_size(counters);
    let blocks_store = Arc::new(
        LocalMemoryOps::new(MAX_SIZE_PER_ITEM_KIB, clock).map_err(LoginThrottleError::from)?,
    );
    blocks_store.set_max_size(blocks);
    Ok(ThrottleStores {
        counters: counters_store,
        blocks: blocks_store,
    })
}

// time.time() for the one deployment that does not inject a clock
#[must_use]
pub fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
    })
}

fn ttl(seconds: i64) -> Result<Ttl, LoginThrottleError> {
    num_traits::cast::<i64, f64>(seconds)
        .map(Some)
        .ok_or(LoginThrottleError::UnrepresentableTtl { seconds })
}

impl LocalStore for LocalMemoryOps {
    fn get(&self, key: &str) -> Result<Option<CacheValue>, LoginThrottleError> {
        self.get_cache(key).map_err(LoginThrottleError::from)
    }

    fn set(&self, key: &str, value: CacheValue, seconds: i64) -> Result<(), LoginThrottleError> {
        self.set_cache(key, value, ttl(seconds)?)
            .map_err(LoginThrottleError::from)
    }

    fn increment(&self, key: &str, seconds: i64) -> Result<f64, LoginThrottleError> {
        self.increment_cache(key, 1.0_f64, ttl(seconds)?)
            .map_err(LoginThrottleError::from)
    }

    fn delete(&self, key: &str) -> Result<(), LoginThrottleError> {
        self.delete_cache(key).map_err(LoginThrottleError::from)
    }
}
