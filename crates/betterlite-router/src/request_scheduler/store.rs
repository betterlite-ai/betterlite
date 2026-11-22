use super::error::SchedulerError;
use super::queue;
use super::service::{MAX_CAS_ATTEMPTS, MemberState, PendingChanges, RequestScheduler};
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
    ttl: Option<f64>,
    carried: &PendingChanges,
) -> Result<Vec<SchedulerTuple>, SharedChange>
where
    F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
{
    let mut expected = Box::pin(shared_read(store, key)).await?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let mut base = queue::members(&expected, model)?;
        // the fallback tier's changes ride along, so this write can neither drop nor revive them
        for (member, state) in carried {
            match state {
                MemberState::Present => {
                    base.insert(member.clone());
                }
                MemberState::Absent => {
                    base.remove(member);
                }
            }
        }
        let next = change(base)?;
        let outcome = Box::pin(swap(store, key, &expected, &next, ttl)).await?;
        if outcome.swapped {
            return Ok(queue::scheduled(&next));
        }
        expected = outcome.current;
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
async fn swap(
    store: &RoutingStateStore,
    key: &RouteKey,
    expected: &StoreRead,
    next: &BTreeSet<String>,
    ttl: Option<f64>,
) -> Result<CompareOutcome, SharedChange> {
    let written = store
        .compare_and_set(key, expected, &RouteValue::Members(next.clone()), ttl)
        .await
        .map_err(|_error| SharedChange::Unavailable(unavailable("compare_and_set")))?;
    let Observed { value, warning } = written;
    warning.map_or(Ok(value), |reason| Err(SharedChange::Unavailable(reason)))
}
impl RequestScheduler {
    // Scheduler.get_queue reads the shared tier first and merges this worker's copy when it answers
    pub(super) async fn queue_members(
        &self,
        model: &str,
    ) -> Result<Observed<BTreeSet<String>>, SchedulerError> {
        let key = queue::key(model)?;
        let Some(shared) = &self.shared else {
            return self.local_members(&key, model, None);
        };
        match Box::pin(shared.get(&key)).await {
            Ok(Observed {
                value,
                warning: None,
            }) => {
                let mut members = queue::members(&value, model)?;
                // a broken local copy cannot hide a shared answer, while a readable one adds the outage's entries
                if let Ok(local) = self.local_members(&key, model, None) {
                    members.extend(local.value);
                }
                Ok(Observed {
                    value: members,
                    warning: None,
                })
            }
            Ok(Observed {
                warning: Some(warning),
                ..
            }) => self.local_members(&key, model, Some(warning)),
            Err(_error) => self.local_members(&key, model, Some(unavailable("async_get_cache"))),
        }
    }
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
    // save_queue against the tier that answered the read
    pub(super) async fn change<F>(
        &self,
        model: &str,
        change: F,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        Box::pin(self.apply_change(model, &change)).await
    }
    // a shared answer mirrors into this worker's copy; a degraded one retries there instead
    async fn apply_change<F>(
        &self,
        model: &str,
        change: &F,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let key = queue::key(model)?;
        let Some(shared) = &self.shared else {
            let (entries, _) = self.change_local(&key, model, change)?;
            return Ok(Observed {
                value: entries,
                warning: None,
            });
        };
        let carried = self.pending_changes(model);
        match Box::pin(cas(shared, &key, model, change, self.local_ttl(), &carried)).await {
            Ok(entries) => {
                self.forget_changes(model, &carried);
                self.mirror_local(&key, &entries)?;
                Ok(Observed {
                    value: entries,
                    warning: None,
                })
            }
            Err(SharedChange::Refused(error)) => Err(error),
            Err(SharedChange::Unavailable(warning)) => {
                let (entries, fallback) = self.change_local(&key, model, change)?;
                self.record_changes(model, fallback);
                Ok(Observed {
                    value: entries,
                    warning: Some(warning),
                })
            }
        }
    }
    // the fallback tier of a degraded shared read: one lock serializes this worker's read-modify-write
    fn change_local<F>(
        &self,
        key: &RouteKey,
        model: &str,
        change: &F,
    ) -> Result<(Vec<SchedulerTuple>, PendingChanges), SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let _guard = self
            .local_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let queued = self.local_members(key, model, None)?;
        let next = change(queued.value.clone())?;
        let mut changes = PendingChanges::new();
        for member in next.difference(&queued.value) {
            changes.insert(member.clone(), MemberState::Present);
        }
        for member in queued.value.difference(&next) {
            changes.insert(member.clone(), MemberState::Absent);
        }
        let entries = queue::scheduled(&next);
        self.local
            .set(key, &RouteValue::Members(next), self.local_ttl())
            .map_err(|source| SchedulerError::Local { source })?;
        Ok((entries, changes))
    }
    // what the fallback tier changed while the shared tier was down, waiting for the next shared write
    fn pending_changes(&self, model: &str) -> PendingChanges {
        let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.get(model).cloned().unwrap_or_default()
    }
    // a fallback change stays recorded, the last one per member, until a shared write carries it over
    fn record_changes(&self, model: &str, changes: PendingChanges) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.entry(model.to_owned()).or_default().extend(changes);
    }
    // a shared write that carried the recorded changes leaves only what changed while it ran
    fn forget_changes(&self, model: &str, carried: &PendingChanges) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(recorded) = pending.get_mut(model) else {
            return;
        };
        recorded.retain(|member, state| match carried.get(member) {
            Some(carried_state) => *state != *carried_state,
            None => true,
        });
        let emptied = recorded.is_empty();
        if emptied {
            pending.remove(model);
        }
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
