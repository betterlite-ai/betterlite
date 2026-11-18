#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use tokio::sync::Notify;

use crate::finops::focus::export_schedule::{FOCUS_USAGE_DATA_JOB_NAME, FocusPodLock};
use crate::finops::focus::support::{self, ExportCall, PodLockCall, Recorder};

#[tokio::test]
async fn an_acquired_lock_runs_the_export_and_releases_it() {
    let (logger, recorder, lock) = support::locked(Recorder::new(false), true, Some(true));
    logger
        .initialize_focus_export_job()
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        lock.calls(),
        vec![
            PodLockCall::Acquire("focus_export_usage_data".to_owned()),
            PodLockCall::Release("focus_export_usage_data".to_owned()),
        ]
    );
    assert_eq!(
        recorder.calls(),
        vec![ExportCall::Window {
            start: support::at("2024-03-10T12:00:00Z"),
            end: support::at("2024-03-10T13:00:00Z"),
            frequency: "hourly".to_owned(),
            limit: None,
        }]
    );
}

// One lease per key, like the redis cache the real manager talks to.
#[derive(Clone, Default)]
struct SharedCache {
    owners: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl LockCache for SharedCache {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        let owner = self.owners.lock().unwrap().get(key).cloned();
        ready(Ok(owner))
    }

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        let mut owners = self.owners.lock().unwrap();
        if owners.contains_key(key) {
            return ready(Ok(false));
        }
        owners.insert(key.to_owned(), owner.as_bytes().to_vec());
        ready(Ok(true))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        let removed = self.owners.lock().unwrap().remove(key);
        ready(Ok(i64::from(removed.is_some())))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn manager(cache: SharedCache) -> Arc<PodLockManager<SharedCache>> {
    Arc::new(PodLockManager::new(
        Some(Arc::new(cache)),
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(uuid::Uuid::new_v4()).expect("the pod id is a v4 UUID"),
    ))
}

#[tokio::test]
async fn another_pod_cannot_export_until_the_holder_finishes_and_releases() {
    let shared = SharedCache::default();
    let holder = manager(shared.clone());
    let other = manager(shared);
    let started = Arc::new(Notify::new());
    let recorder = Recorder::paused(false, Arc::clone(&started));
    let pod_lock: Arc<dyn FocusPodLock> = holder;
    let logger = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        Some(pod_lock),
    ));
    let job = tokio::spawn({
        let logger = Arc::clone(&logger);
        async move { logger.initialize_focus_export_job().await }
    });
    started.notified().await;
    assert_eq!(
        other
            .acquire_lock(FOCUS_USAGE_DATA_JOB_NAME, None, false)
            .await,
        Some(false)
    );
    job.abort();
    let cancelled = job.await.expect_err("the export is cancelled");
    assert!(cancelled.is_cancelled());
    logger.drain().await;
    assert_eq!(
        other
            .acquire_lock(FOCUS_USAGE_DATA_JOB_NAME, None, false)
            .await,
        Some(true)
    );
    assert_eq!(recorder.calls().len(), 1);
}
