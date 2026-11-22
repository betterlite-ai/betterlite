use super::error::SchedulerError;
use super::queue;
use super::service::{MAX_CAS_ATTEMPTS, RequestScheduler};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StoreWarning};
use crate::routing_value_store::{CompareOutcome, RouteKey, RouteValue, SchedulerTuple};
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
    let read = store
        .get(key)
        .await
        .map_err(|_error| SharedChange::Unavailable(unavailable("async_get_cache")))?;
    let mut expected = match read {
        Observed {
            value,
            warning: None,
        } => value,
        Observed {
            warning: Some(warning),
            ..
        } => return Err(SharedChange::Unavailable(warning)),
    };
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let next = change(queue::members(&expected, model)?)?;
        let outcome = store
            .compare_and_set(key, &expected, &RouteValue::Members(next.clone()), None)
            .await
            .map_err(|_error| SharedChange::Unavailable(unavailable("compare_and_set")))?;
        match outcome {
            Observed {
                value: CompareOutcome { swapped: true, .. },
                warning: None,
            } => return Ok(queue::scheduled(&next)),
            Observed {
                value:
                    CompareOutcome {
                        swapped: false,
                        current,
                    },
                warning: None,
            } => expected = current,
            Observed {
                warning: Some(warning),
                ..
            } => return Err(SharedChange::Unavailable(warning)),
        }
    }
    Err(SharedChange::Refused(SchedulerError::Contended {
        model: model.to_owned(),
        attempts: MAX_CAS_ATTEMPTS,
    }))
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
            Ok(observed) if observed.warning.is_none() => Ok(Observed {
                value: queue::members(&observed.value, model)?,
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
