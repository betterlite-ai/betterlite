use super::error::SchedulerError;
use super::queue;
use super::service::{MAX_CAS_ATTEMPTS, RequestScheduler};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StoreWarning};
use crate::routing_value_store::{CompareOutcome, RouteKey, RouteValue, SchedulerTuple, StoreRead};
use std::collections::BTreeSet;
use std::sync::PoisonError;
// why the shared tier did not take an update: a degraded answer falls back to this worker's copy, a refused change stays an error
enum SharedChange {
    Unavailable(StoreWarning),
    Refused(SchedulerError),
}
impl From<SchedulerError> for SharedChange {
    fn from(error: SchedulerError) -> Self {
        Self::Refused(error)
    }
}
// one retry loop for the shared tier: a lost compare-and-set re-reads and decides again
async fn cas<F>(
    store: &RoutingStateStore,
    key: &RouteKey,
    model: &str,
    change: &F,
) -> Result<Vec<SchedulerTuple>, SharedChange>
where
    F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
{
    let mut expected = shared_read(store, key).await?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let next = change(queue::members(&expected, model)?)?;
        match swap(store, key, &expected, &next).await? {
            CompareOutcome { swapped: true, .. } => return Ok(queue::scheduled(&next)),
            CompareOutcome {
                swapped: false,
                current,
            } => expected = current,
        }
    }
    Err(SharedChange::Refused(SchedulerError::Contended {
        model: model.to_owned(),
        attempts: MAX_CAS_ATTEMPTS,
    }))
}
// the shared read: a warning is this worker's observation, so the caller falls back to its own copy
async fn shared_read(store: &RoutingStateStore, key: &RouteKey) -> Result<StoreRead, SharedChange> {
    let read = store
        .get(key)
        .await
        .map_err(|_error| SharedChange::Unavailable(unavailable("async_get_cache")))?;
    match read {
        Observed {
            value,
            warning: None,
        } => Ok(value),
        Observed {
            warning: Some(warning),
            ..
        } => Err(SharedChange::Unavailable(warning)),
    }
}
// the compare-and-set: the outcome says whether this writer won or read the winner's value
async fn swap(
    store: &RoutingStateStore,
    key: &RouteKey,
    expected: &StoreRead,
    next: &BTreeSet<String>,
) -> Result<CompareOutcome, SharedChange> {
    let written = store
        .compare_and_set(key, expected, &RouteValue::Members(next.clone()), None)
        .await
        .map_err(|_error| SharedChange::Unavailable(unavailable("compare_and_set")))?;
    let Observed { value, warning } = written;
    warning.map_or(Ok(value), |reason| Err(SharedChange::Unavailable(reason)))
}
impl RequestScheduler {
    // Scheduler.get_queue reads the shared tier first and this worker's copy when it cannot answer
    pub(super) async fn queue_members(
        &self,
        model: &str,
    ) -> Result<Observed<BTreeSet<String>>, SchedulerError> {
        let key = queue::key(model)?;
        let Some(shared) = &self.shared else {
            return self.local_members(&key, model, None);
        };
        match shared.get(&key).await {
            Ok(Observed {
                value,
                warning: None,
            }) => Ok(Observed {
                value: queue::members(&value, model)?,
                warning: None,
            }),
            Ok(Observed {
                warning: Some(warning),
                ..
            }) => self.local_members(&key, model, Some(warning)),
            Err(_error) => self.local_members(&key, model, Some(unavailable("async_get_cache"))),
        }
    }
    // this worker's copy answers when the shared tier is absent or could not be read
    fn local_members(
        &self,
        key: &RouteKey,
        model: &str,
        warning: Option<StoreWarning>,
    ) -> Result<Observed<BTreeSet<String>>, SchedulerError> {
        let read = self
            .local
            .get(key)
            .map_err(|source| SchedulerError::Local { source })?;
        Ok(Observed {
            value: queue::members(&read, model)?,
            warning,
        })
    }
    // save_queue against the tier that answered the read, retried while another writer wins the compare
    pub(super) async fn change<F>(
        &self,
        model: &str,
        change: F,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let key = queue::key(model)?;
        let Some(shared) = &self.shared else {
            return Ok(Observed {
                value: self.change_local(&key, model, &change)?,
                warning: None,
            });
        };
        self.change_shared(shared, &key, model, &change).await
    }
    // a shared answer mirrors into this worker's copy; a degraded one retries there instead
    async fn change_shared<F>(
        &self,
        shared: &RoutingStateStore,
        key: &RouteKey,
        model: &str,
        change: &F,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        match cas(shared, key, model, change).await {
            Ok(entries) => {
                self.mirror_local(key, &entries)?;
                Ok(Observed {
                    value: entries,
                    warning: None,
                })
            }
            Err(SharedChange::Unavailable(warning)) => Ok(Observed {
                value: self.change_local(key, model, change)?,
                warning: Some(warning),
            }),
            Err(SharedChange::Refused(error)) => Err(error),
        }
    }
    // the fallback tier of a degraded shared read: one lock serializes this worker's read-modify-write
    fn change_local<F>(
        &self,
        key: &RouteKey,
        model: &str,
        change: &F,
    ) -> Result<Vec<SchedulerTuple>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let _guard = self
            .local_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let queued = self.local_members(key, model, None)?;
        let next = change(queued.value)?;
        let entries = queue::scheduled(&next);
        self.local
            .set(key, &RouteValue::Members(next), self.local_ttl())
            .map_err(|source| SchedulerError::Local { source })?;
        Ok(entries)
    }
    // DualCache.async_set_cache writes both tiers, so a shared answer leaves the same queue in this worker's copy
    fn mirror_local(
        &self,
        key: &RouteKey,
        entries: &[SchedulerTuple],
    ) -> Result<(), SchedulerError> {
        let _guard = self
            .local_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let members = RouteValue::Members(queue::encoded(entries));
        self.local
            .set(key, &members, self.local_ttl())
            .map_err(|source| SchedulerError::Local { source })
    }
    // Scheduler.__init__ passes DualCache its in-memory ttl only when a redis cache is attached
    fn local_ttl(&self) -> Option<f64> {
        self.shared
            .is_some()
            .then_some(self.settings.local_ttl_seconds)
    }
}
// a shared tier that answered with an error is this worker's observation only, so the queue keeps serving from its own copy
const fn unavailable(operation: &'static str) -> StoreWarning {
    StoreWarning::Unavailable {
        operation,
        event: None,
    }
}
