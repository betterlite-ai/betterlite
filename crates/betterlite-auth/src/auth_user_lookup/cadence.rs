use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_cache::dual_cache::limited_size::LimitedSizeOrderedDict;
use betterlite_db::budget_permission_rows::JsonData;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

// auth_checks.py's time.time(): _should_check_db and _update_last_db_access_time sample it themselves
pub trait CadenceClock: Send + Sync {
    fn now(&self) -> Duration;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WallClock;

impl CadenceClock for WallClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CadenceEntry {
    pub record: Option<JsonData>,
    pub checked_at: Duration,
}

// auth_checks.py L395: last_db_access_time, keyed by user_id:{requested_id}
pub struct Cadence {
    pub(super) entries: Mutex<LimitedSizeOrderedDict<CadenceEntry>>,
    clock: Arc<dyn CadenceClock>,
}

impl Cadence {
    #[must_use]
    pub fn new(max_size: usize, clock: Arc<dyn CadenceClock>) -> Self {
        Self {
            entries: Mutex::new(LimitedSizeOrderedDict {
                entries: IndexMap::new(),
                max_size,
            }),
            clock,
        }
    }

    // _should_check_db: an absent key or any stored value reads; a stored None waits out the expiry
    #[must_use]
    pub fn should_check_db(&self, key: &str, expiry: i64) -> bool {
        let now = self.clock.now();
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.entries.get(key) {
            None => true,
            Some(entry) if entry.record.is_some() => true,
            Some(entry) => expired(now, entry.checked_at, expiry),
        }
    }

    // _update_last_db_access_time: a write replaces the value and samples a fresh time
    pub fn update(&self, key: &str, record: Option<JsonData>) {
        let checked_at = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.insert(key.to_owned(), CadenceEntry { record, checked_at });
    }
}

// current_time - last_db_access_time[key][1] >= db_cache_expiry, without losing a backward clock's sign
fn expired(now: Duration, checked_at: Duration, expiry: i64) -> bool {
    match now.checked_duration_since(checked_at) {
        Some(elapsed) => {
            elapsed.as_secs_f64() >= Decimal::from(expiry).to_f64().unwrap_or(f64::INFINITY)
        }
        None => {
            expiry.is_negative()
                && checked_at.duration_since(now).as_secs_f64() <= magnitude_seconds(expiry)
        }
    }
}

fn magnitude_seconds(expiry: i64) -> f64 {
    Decimal::from(expiry.unsigned_abs())
        .to_f64()
        .unwrap_or(f64::INFINITY)
}
