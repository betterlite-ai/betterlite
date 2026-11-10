use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};

use lru::LruCache;

use super::auth::{MonotonicClock, WorkloadIdentityAuth};
use super::config::{OpenAiWorkloadIdentityConfig, SdkCapability};
use super::error::WorkloadIdentityError;
use super::transport::WorkloadIdentityTransport;

// functools.lru_cache(maxsize=16)
pub const AUTH_CACHE_ENTRIES: usize = 16;

pub struct WorkloadIdentityCredentials<T> {
    auths: Mutex<LruCache<OpenAiWorkloadIdentityConfig, Arc<WorkloadIdentityAuth<T>>>>,
    transport: Arc<T>,
    clock: MonotonicClock,
    sdk: SdkCapability,
}

impl<T: WorkloadIdentityTransport> WorkloadIdentityCredentials<T> {
    pub fn new(transport: Arc<T>, clock: MonotonicClock, sdk: SdkCapability) -> Self {
        let capacity = NonZeroUsize::new(AUTH_CACHE_ENTRIES).unwrap_or(NonZeroUsize::MIN);
        Self {
            auths: Mutex::new(LruCache::new(capacity)),
            transport,
            clock,
            sdk,
        }
    }

    pub fn workload_identity_auth(
        &self,
        config: &OpenAiWorkloadIdentityConfig,
    ) -> Result<Arc<WorkloadIdentityAuth<T>>, WorkloadIdentityError> {
        let cached = self
            .auths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(config)
            .map(Arc::clone);
        if let Some(auth) = cached {
            return Ok(auth);
        }
        self.sdk.load_workload_identity_auth()?;
        let workload_identity = config.to_sdk_workload_identity(self.sdk)?;
        let built = Arc::new(WorkloadIdentityAuth::new(
            workload_identity,
            Arc::clone(&self.transport),
            Arc::clone(&self.clock),
        ));
        let mut auths = self.auths.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(auths.get_or_insert(config.clone(), || built)))
    }
}
