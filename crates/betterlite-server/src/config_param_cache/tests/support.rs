use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::table_ops::TableOpsError;
use betterlite_db::table_ops::rows::ConfigParamRow;

use crate::config_param_cache::ConfigParamCache;
use crate::config_param_cache::store::ConfigParamStore;

fn lookup<T>(map: &Mutex<BTreeMap<String, T>>, key: &str) -> Option<T>
where
    T: Clone,
{
    map.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) rows: Mutex<BTreeMap<String, ConfigParamRow>>,
    pub(super) users: Mutex<BTreeMap<String, LiteLLMUserTable>>,
    pub(super) param_reads: AtomicUsize,
    pub(super) batch_reads: AtomicUsize,
    pub(super) user_reads: AtomicUsize,
    pub(super) batch_fails: AtomicBool,
    pub(super) last_batch: Mutex<Vec<String>>,
}

impl FakeStore {
    pub(super) fn add_param(&self, name: &str, value: &str) {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), config_row(name, value));
    }

    pub(super) fn add_user(&self, user_id: &str) {
        self.users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned(), user_row(user_id));
    }

    pub(super) fn fail_batches(&self) {
        self.batch_fails.store(true, Ordering::SeqCst);
    }

    pub(super) fn param_calls(&self) -> usize {
        self.param_reads.load(Ordering::SeqCst)
    }

    pub(super) fn batch_calls(&self) -> usize {
        self.batch_reads.load(Ordering::SeqCst)
    }

    pub(super) fn user_calls(&self) -> usize {
        self.user_reads.load(Ordering::SeqCst)
    }
}

impl ConfigParamStore for FakeStore {
    type ClientError = Infallible;

    fn find_param(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<ConfigParamRow>, TableOpsError<Infallible>>> + Send
    {
        self.param_reads.fetch_add(1, Ordering::SeqCst);
        ready(Ok(lookup(&self.rows, param_name)))
    }

    fn find_params(
        &self,
        param_names: &[String],
    ) -> impl Future<Output = Result<Vec<ConfigParamRow>, TableOpsError<Infallible>>> + Send {
        self.batch_reads.fetch_add(1, Ordering::SeqCst);
        *self
            .last_batch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = param_names.to_vec();
        if self.batch_fails.load(Ordering::SeqCst) {
            return ready(Err(TableOpsError::NoDbConnected));
        }
        ready(Ok(param_names
            .iter()
            .filter_map(|name| lookup(&self.rows, name))
            .collect()))
    }

    fn user_row(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, TableOpsError<Infallible>>> + Send
    {
        self.user_reads.fetch_add(1, Ordering::SeqCst);
        ready(Ok(lookup(&self.users, user_id)))
    }
}

pub(super) struct Harness {
    pub(super) now: Arc<AtomicU32>,
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

    pub(super) fn cache(&self, ttl_seconds: f64) -> ConfigParamCache<'_, LocalMemoryOps> {
        ConfigParamCache {
            cache: &self.dual,
            ttl_seconds,
        }
    }

    pub(super) fn advance(&self, seconds: u32) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }
}

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the JSON parses")
}

pub(super) fn config_row(name: &str, value: &str) -> ConfigParamRow {
    let text = format!(r#"{{"param_name":"{name}","param_value":{value}}}"#);
    serde_json::from_str(&text).expect("the config row parses")
}

pub(super) fn user_row(user_id: &str) -> LiteLLMUserTable {
    let text = format!(r#"{{"user_id":"{user_id}"}}"#);
    serde_json::from_str(&text).expect("the user row parses")
}
