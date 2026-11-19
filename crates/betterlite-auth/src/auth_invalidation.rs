pub mod clock;
pub mod error;
pub mod eviction;
pub mod keys;
pub mod member;
pub mod ports;
pub mod refresh;
pub mod registry;
pub mod transport;

mod tests;

use std::sync::Arc;

use self::clock::InvalidationClock;
use self::ports::{AuthInvalidationCache, SpendCounterCache, UsageInvalidationCache};
use self::transport::InvalidationTransport;

// proxy_server.py's SPEND_DB_FLOOR_CACHE_TTL_SECONDS; the authoritative-floor reader reads the same value
pub const SPEND_DB_FLOOR_TTL_SECONDS: f64 = 5.0;
// the reset writes the live counter for a minute, the same window /key/{key}/reset_spend has used
pub const SPEND_COUNTER_TTL_SECONDS: f64 = 60.0;

pub struct AuthInvalidation {
    pub auth: Arc<dyn AuthInvalidationCache>,
    pub usage: Option<Arc<dyn UsageInvalidationCache>>,
    pub spend: Arc<dyn SpendCounterCache>,
    pub transport: Arc<dyn InvalidationTransport>,
    pub clock: Arc<dyn InvalidationClock>,
    pub access_group_ttl: f64,
}
