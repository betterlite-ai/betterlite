#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::queues::pod_lock::owner::lease_key;
use betterlite_db::queues::pod_lock::{LockCache, LockCacheError, LockScript};
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_shared::background::DeploymentSource;

#[derive(Default)]
pub(crate) struct FakeLock {
    values: Mutex<BTreeMap<String, String>>,
    deletes: Mutex<Vec<String>>,
    ttls: Mutex<Vec<i64>>,
}

impl FakeLock {
    pub(crate) fn acquire_ttls(&self) -> Vec<i64> {
        self.ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn holds(&self, job: &str) -> bool {
        let key = lease_key(job);
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&key)
    }

    pub(crate) fn released(&self, job: &str) -> bool {
        let key = lease_key(job);
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|deleted| deleted == &key)
    }
}

impl LockCache for FakeLock {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        let owner = self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|owner| owner.as_bytes().to_vec());
        ready(Ok(owner))
    }

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.contains_key(key) {
            return ready(Ok(false));
        }
        values.insert(key.to_owned(), owner.to_owned());
        drop(values);
        self.ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ttl_seconds);
        ready(Ok(true))
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        let mut deletes = self.deletes.lock().unwrap_or_else(PoisonError::into_inner);
        deletes.push(key.to_owned());
        drop(deletes);
        let removed = self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key)
            .is_some();
        ready(Ok(i64::from(removed)))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

#[derive(Default)]
pub(crate) struct FakeDeployments {
    deployments: Mutex<Option<Vec<DeploymentSpec>>>,
}

impl FakeDeployments {
    pub(crate) fn new(deployments: Option<Vec<DeploymentSpec>>) -> Arc<Self> {
        Arc::new(Self {
            deployments: Mutex::new(deployments),
        })
    }

    pub(crate) fn set(&self, deployments: Vec<DeploymentSpec>) {
        *self
            .deployments
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(deployments);
    }
}

impl DeploymentSource for FakeDeployments {
    fn snapshot(&self) -> Option<Vec<DeploymentSpec>> {
        self.deployments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
