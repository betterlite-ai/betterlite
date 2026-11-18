#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use uuid::Uuid;

use crate::finops::cloudzero::schedule::lease::UsageExportLease;

const JOB: &str = "cloudzero_export_usage_data";
const LEASE_KEY: &str = "cronjob_lock:cloudzero_export_usage_data";

#[derive(Default)]
struct FakeCache {
    owners: Mutex<BTreeMap<String, Vec<u8>>>,
    deleted: Mutex<Vec<String>>,
}

impl LockCache for FakeCache {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        match self.owners.lock().expect("an unsplit lock").get(key) {
            Some(owner) => ready(Ok(Some(owner.clone()))),
            None => ready(Ok(None)),
        }
    }

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        let mut owners = self.owners.lock().expect("an unsplit lock");
        let inserted = if owners.contains_key(key) {
            false
        } else {
            owners.insert(key.to_owned(), owner.as_bytes().to_vec());
            true
        };
        ready(Ok(inserted))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        let removed = self.owners.lock().expect("an unsplit lock").remove(key);
        if removed.is_none() {
            return ready(Ok(0));
        }
        self.deleted
            .lock()
            .expect("an unsplit lock")
            .push(key.to_owned());
        ready(Ok(1))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn manager(cache: Option<Arc<FakeCache>>) -> PodLockManager<FakeCache> {
    let pod_id = PodId::try_from(Uuid::new_v4()).expect("a fresh pod id");
    PodLockManager::new(cache, 300, Arc::new(NoPodLockEventSink), pod_id)
}

fn lease_object(manager: PodLockManager<FakeCache>) -> Arc<dyn UsageExportLease> {
    Arc::new(manager)
}

mod without_cache {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let lease = lease_object(manager(None));
        assert!(!lease.redis_available());
        assert_eq!(lease.acquire_lock(JOB).await, None);
        lease.release_lock(JOB).await;
    }
}

mod with_cache {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let cache = Arc::new(FakeCache::default());
        let lease = lease_object(manager(Some(Arc::clone(&cache))));
        assert!(lease.redis_available());
        assert_eq!(lease.acquire_lock(JOB).await, Some(true));
        assert_eq!(lease.acquire_lock(JOB).await, Some(true));
        lease.release_lock(JOB).await;
        let deleted = cache.deleted.lock().expect("an unsplit lock").clone();
        assert_eq!(deleted, vec![LEASE_KEY.to_owned()]);
    }
}
