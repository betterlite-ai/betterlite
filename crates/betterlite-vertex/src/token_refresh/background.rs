use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;
use tokio_util::task::TaskTracker;
use tracing::warn;

use crate::token_cache::CredentialCache;
use crate::token_cache::key::CredentialKey;
use crate::token_cache::record::{CacheRecord, CredentialSnapshot};

pub(crate) struct RefreshJob {
    pub(super) id: u64,
    pub(super) done: watch::Receiver<bool>,
}

// LiteLLM's _schedule_background_refresh: one refresh per identity, replaced once its task has finished
pub(super) fn schedule_background_refresh(
    cache: &CredentialCache,
    tasks: &TaskTracker,
    credential: Arc<CredentialSnapshot>,
    key: CredentialKey,
    project: Option<String>,
) {
    if tasks.is_closed() {
        return;
    }
    let mut jobs = cache
        .refreshes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if jobs.get(&key).is_some_and(|job| !*job.done.borrow()) {
        return;
    }
    let id = cache.next_refresh.fetch_add(1, Ordering::Relaxed);
    let (sender, done) = watch::channel(false);
    jobs.insert(key.clone(), RefreshJob { id, done });
    drop(jobs);
    let executor = Arc::clone(&cache.executor);
    let records = Arc::clone(&cache.records);
    let tracked = Arc::clone(&cache.refreshes);
    tasks.spawn(async move {
        match executor.refresh(&credential).await {
            Ok(snapshot) => store_refreshed(&records, &key, &credential, snapshot, project),
            Err(error) => warn!(error = %error, "background vertex credential refresh failed"),
        }
        sender.send_replace(true);
        let mut tracked_jobs = tracked.lock().unwrap_or_else(PoisonError::into_inner);
        release_job(&mut tracked_jobs, &key, id);
    });
}

// a finishing task removes only its own entry, never the replacement a later request scheduled
pub(super) fn release_job(
    jobs: &mut BTreeMap<CredentialKey, RefreshJob>,
    key: &CredentialKey,
    id: u64,
) {
    if jobs.get(key).is_some_and(|job| job.id == id) {
        jobs.remove(key);
    }
}

// LiteLLM's _await_in_flight_background_refresh: a blocking refresh waits for the background one it would race
pub(super) async fn await_background_refresh(cache: &CredentialCache, key: &CredentialKey) {
    let mut done = {
        let jobs = cache
            .refreshes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match jobs.get(key) {
            Some(job) => job.done.clone(),
            None => return,
        }
    };
    if *done.borrow() {
        return;
    }
    let _changed = done.changed().await;
}

// LiteLLM writes the result only while the slot still holds the credentials object it refreshed
fn store_refreshed(
    records: &Mutex<BTreeMap<CredentialKey, CacheRecord>>,
    key: &CredentialKey,
    handle: &Arc<CredentialSnapshot>,
    snapshot: CredentialSnapshot,
    project: Option<String>,
) {
    let mut cached = records.lock().unwrap_or_else(PoisonError::into_inner);
    let holds_handle = cached
        .get(key)
        .is_some_and(|record| Arc::ptr_eq(record.credential(), handle));
    if !holds_handle {
        return;
    }
    cached.insert(
        key.clone(),
        CacheRecord::Paired {
            credential: Arc::new(snapshot),
            project,
        },
    );
}
