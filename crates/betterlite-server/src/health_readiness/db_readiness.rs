use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::database_availability::policy::{
    AllowRequestsOnOutage, allows_requests_during_outage,
};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_faults::{DbError, is_database_transport_error};
use tokio::time::{Instant, timeout};

use crate::health_readiness::readiness_details::ReadinessSources;

mod tests;

const CACHE_TTL: Duration = Duration::from_secs(15);
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_DEADLINE: Duration = Duration::from_secs(4);
const RECONNECT_REASON: &str = "health_readiness_check";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbStatus {
    Unknown,
    Connected,
    Disconnected,
    NotConnected,
    Stalled,
}

impl DbStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::NotConnected => "Not connected",
            Self::Stalled => "stalled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DbHealth {
    pub status: DbStatus,
    pub last_updated: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicDbReport {
    pub status: DbStatus,
    pub unavailable: bool,
}

#[async_trait]
pub trait DatabaseProbe: Send + Sync {
    fn configured(&self) -> bool;
    async fn health_check(&self) -> Result<(), DbError>;
    async fn attempt_reconnect(
        &self,
        reason: &str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Result<bool, DbError>;
    async fn live_worker_count(&self) -> Option<u32>;
}

pub trait StallTracker: Send + Sync {
    fn stalled_within_window(&self, window_seconds: f64) -> bool;
}

impl StallTracker for DbLookupStallTracker<SystemClock> {
    fn stalled_within_window(&self, window_seconds: f64) -> bool {
        self.stalled_within(window_seconds)
    }
}

pub struct DbReadiness {
    probe: Arc<dyn DatabaseProbe>,
    stall: Arc<dyn StallTracker>,
    availability: AllowRequestsOnOutage,
    stall_window_seconds: f64,
    disable_no_redis_warning: bool,
    cache: Mutex<DbHealth>,
}

impl DbReadiness {
    #[must_use]
    pub fn new(
        probe: Arc<dyn DatabaseProbe>,
        stall: Arc<dyn StallTracker>,
        availability: AllowRequestsOnOutage,
        stall_window_seconds: f64,
        disable_no_redis_warning: bool,
    ) -> Self {
        Self {
            probe,
            stall,
            availability,
            stall_window_seconds,
            disable_no_redis_warning,
            cache: Mutex::new(DbHealth {
                status: DbStatus::Unknown,
                last_updated: Instant::now(),
            }),
        }
    }

    fn cached(&self) -> DbHealth {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn store(&self, status: DbStatus) -> DbHealth {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.status = status;
        cache.last_updated = Instant::now();
        cache.clone()
    }

    pub async fn check(&self) -> DbHealth {
        match timeout(PROBE_DEADLINE, self.check_unbounded()).await {
            Ok(health) => health,
            // the deadline reports the cached instant and writes nothing, so one slow probe poisons no later one
            Err(_elapsed) => {
                let cached = self.cached();
                DbHealth {
                    status: DbStatus::Disconnected,
                    last_updated: cached.last_updated,
                }
            }
        }
    }

    async fn check_unbounded(&self) -> DbHealth {
        let cached = self.cached();
        if cached.status == DbStatus::Connected && cached.last_updated.elapsed() < CACHE_TTL {
            return cached;
        }
        if !self.probe.configured() {
            return self.store(DbStatus::Disconnected);
        }
        match timeout(CHECK_TIMEOUT, self.probe.health_check()).await {
            Ok(Ok(())) => self.store(DbStatus::Connected),
            Ok(Err(error)) => {
                let disconnected = self.store(DbStatus::Disconnected);
                // a data-layer failure means the database answered, and a 2 s timeout is not a transport error
                if !is_database_transport_error(&error) {
                    return disconnected;
                }
                tracing::warn!(
                    "_db_health_readiness_check: health_check failed, attempting reconnect"
                );
                self.reconnect().await
            }
            Err(_elapsed) => self.store(DbStatus::Disconnected),
        }
    }

    async fn reconnect(&self) -> DbHealth {
        let reconnected = self
            .probe
            .attempt_reconnect(
                RECONNECT_REASON,
                CHECK_TIMEOUT.as_secs_f64(),
                CHECK_TIMEOUT.as_secs_f64(),
            )
            .await;
        // LiteLLM ignores the false a skipped reconnect returns and asks the ping again either way
        let succeeded = match reconnected {
            Ok(_attempted) => matches!(
                timeout(CHECK_TIMEOUT, self.probe.health_check()).await,
                Ok(Ok(()))
            ),
            Err(_error) => false,
        };
        if succeeded {
            tracing::info!("_db_health_readiness_check: reconnect succeeded");
            return self.store(DbStatus::Connected);
        }
        tracing::error!("_db_health_readiness_check: reconnect failed");
        self.cached()
    }

    #[must_use]
    pub fn readiness_db_status(&self, status: DbStatus) -> DbStatus {
        match status {
            DbStatus::Connected if self.stall.stalled_within_window(self.stall_window_seconds) => {
                DbStatus::Stalled
            }
            other => other,
        }
    }

    pub async fn resolve_public_db(&self) -> PublicDbReport {
        if !self.probe.configured() {
            return PublicDbReport {
                status: DbStatus::NotConnected,
                unavailable: false,
            };
        }
        let status = self.readiness_db_status(self.check().await.status);
        // LiteLLM's str_to_bool reads an unreadable policy as false, so a refused policy does not allow
        let outage_allowed = matches!(allows_requests_during_outage(&self.availability), Ok(true));
        let unavailable = status != DbStatus::Connected && !outage_allowed;
        PublicDbReport {
            status,
            unavailable,
        }
    }

    pub async fn show_no_redis_warning(&self, sources: &dyn ReadinessSources) -> bool {
        if sources.redis_configured() {
            return false;
        }
        if self.disable_no_redis_warning {
            return false;
        }
        if !self.probe.configured() {
            return true;
        }
        // a census that failed (None) is not one worker, so the warning stands
        self.probe.live_worker_count().await != Some(1)
    }
}
