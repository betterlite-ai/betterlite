use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheSettings};
use betterlite_config::sections::settings::settings_rules::JsonValue;
use betterlite_config::ui_settings::UiSettingsDict;
use betterlite_db::auxiliary_repository::repository::TableError;

use crate::jobs::ptu::attribution::PtuAttribution;
use crate::ui_settings_state::derived::PtuCostFlag;
use crate::ui_settings_state::repository::UiSettingsRepository;

pub(super) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the JSON parses")
}

pub(super) fn settings(text: &str) -> UiSettingsDict {
    serde_json::from_str(text).expect("the settings parse")
}

pub(super) fn stored(text: &str) -> String {
    text.to_owned()
}

pub(super) struct FakeRepository {
    row: Mutex<Option<String>>,
    reads: AtomicUsize,
    fails: AtomicBool,
}

impl FakeRepository {
    pub(super) const fn new(row: Option<String>) -> Self {
        Self {
            row: Mutex::new(row),
            reads: AtomicUsize::new(0),
            fails: AtomicBool::new(false),
        }
    }

    pub(super) fn deny_reads(&self) {
        self.fails.store(true, Ordering::SeqCst);
    }

    pub(super) fn remove_row(&self) {
        *self.row.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub(super) fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl UiSettingsRepository for FakeRepository {
    async fn stored_ui_settings(&self) -> Result<Option<String>, TableError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fails.load(Ordering::SeqCst) {
            return Err(TableError::Row(
                serde_json::from_str::<UiSettingsDict>("{")
                    .expect_err("the stored row is malformed"),
            ));
        }
        Ok(self
            .row
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }
}

pub(super) struct FakePtuFlag {
    pub(super) enabled: bool,
    pub(super) configured: bool,
}

impl PtuAttribution for FakePtuFlag {
    fn enabled(&self) -> bool {
        self.enabled
    }
}

impl PtuCostFlag for FakePtuFlag {
    fn configured(&self) -> bool {
        self.configured
    }
}

pub(super) struct Harness {
    now: Arc<AtomicU32>,
    pub(super) dual: DualCache<LocalMemoryOps>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let now = Arc::new(AtomicU32::new(1_000));
        let clock_state = Arc::clone(&now);
        let clock_of: Arc<dyn Fn() -> f64 + Send + Sync> =
            Arc::new(move || f64::from(clock_state.load(Ordering::SeqCst)));
        let memory = Arc::new(LocalMemoryOps::new(1024, clock_of).expect("the memory tier builds"));
        let dual = DualCache::new(
            memory,
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 1000,
                redis_delete_chunk_size: 1000,
            },
        );
        Self { now, dual }
    }

    pub(super) fn advance(&self, seconds: u32) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }
}

pub(super) const fn cache_options() -> CacheOptions {
    CacheOptions {
        ttl: Some(600.0),
        local_only: false,
        refresh_ttl: false,
    }
}
