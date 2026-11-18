#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};

use crate::finops::focus::export_schedule::FocusPodLock;

struct MemorylessCache;

impl LockCache for MemorylessCache {
    fn get_owner(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        ready(Ok(None))
    }

    fn set_owner_if_absent(
        &self,
        _key: &str,
        _owner: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        ready(Ok(true))
    }

    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        ready(Ok(1))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn manager(cache: Option<Arc<MemorylessCache>>) -> PodLockManager<MemorylessCache> {
    PodLockManager::new(
        cache,
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(uuid::Uuid::new_v4()).expect("the pod id is a v4 UUID"),
    )
}

#[test]
fn the_port_reports_whether_a_cache_is_configured() {
    assert!(!manager(None).has_cache());
    assert!(manager(Some(Arc::new(MemorylessCache))).has_cache());
}
