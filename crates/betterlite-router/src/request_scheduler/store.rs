use super::error::SchedulerError;
use super::queue;
use super::service::{MAX_CAS_ATTEMPTS, RequestScheduler};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StoreWarning};
use crate::routing_value_store::{CompareOutcome, RouteKey, RouteValue, SchedulerTuple, StoreRead};
use std::collections::BTreeSet;
// why the shared tier did not take an update: a degraded read falls back to this worker's copy, a refused change stays an error
enum SharedChange {
    Unavailable(StoreWarning),
    Refused(SchedulerError),
}
impl From<SchedulerError> for SharedChange {
    fn from(error: SchedulerError) -> Self {
        Self::Refused(error)
    }
}
impl RequestScheduler {
    // Scheduler.get_queue reads the shared tier first and this worker's copy when it cannot answer
    pub(super) async fn queue_members(
        &self,
        model: &str,
    ) -> Result<Observed<BTreeSet<String>>, SchedulerError> {
        let key = queue::key(model)?;
        let (read, warning) = self.read(&key).await?;
        Ok(Observed {
            value: queue::members(&read, model)?,
            warning,
        })
    }
    async fn read(
        &self,
        key: &RouteKey,
    ) -> Result<(StoreRead, Option<StoreWarning>), SchedulerError> {
        let Some(shared) = &self.shared else {
            return self.read_local(key, None);
        };
        match shared.get(key).await {
            Ok(Observed {
                value: _stale,
                warning: Some(warning),
            }) => self.read_local(key, Some(warning)),
            Ok(Observed {
                value,
                warning: None,
            }) => Ok((value, None)),
            Err(_error) => self.read_local(key, Some(unavailable("async_get_cache"))),
        }
    }
    // this worker's copy answers when the shared tier is absent or could not be read
    fn read_local(
        &self,
        key: &RouteKey,
        warning: Option<StoreWarning>,
    ) -> Result<(StoreRead, Option<StoreWarning>), SchedulerError> {
        let read = self
            .local
            .get(key)
            .map_err(|source| SchedulerError::Local { source })?;
        Ok((read, warning))
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
            return self.change_local(&key, model, &change, None);
        };
        match self.change_shared(shared, &key, model, &change).await {
            Ok(changed) => {
                self.mirror_local(&key, &changed.value)?;
                Ok(changed)
            }
            Err(SharedChange::Unavailable(warning)) => {
                self.change_local(&key, model, &change, Some(warning))
            }
            Err(SharedChange::Refused(error)) => Err(error),
        }
    }
    // DualCache.async_set_cache writes both tiers, so a shared answer leaves the same queue in this worker's copy
    fn mirror_local(
        &self,
        key: &RouteKey,
        entries: &[SchedulerTuple],
    ) -> Result<(), SchedulerError> {
        self.local
            .set(key, &encoded(entries), self.local_ttl())
            .map_err(|source| SchedulerError::Local { source })
    }
    async fn change_shared<F>(
        &self,
        shared: &RoutingStateStore,
        key: &RouteKey,
        model: &str,
        change: &F,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SharedChange>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let mut expected = match shared.get(key).await {
            Ok(Observed {
                value,
                warning: None,
            }) => value,
            Ok(Observed {
                value: _stale,
                warning: Some(warning),
            }) => return Err(SharedChange::Unavailable(warning)),
            Err(_error) => return Err(SharedChange::Unavailable(unavailable("async_get_cache"))),
        };
        for _attempt in 0..MAX_CAS_ATTEMPTS {
            let next = change(queue::members(&expected, model)?)?;
            let outcome = shared
                .compare_and_set(key, &expected, &RouteValue::Members(next.clone()), None)
                .await;
            match outcome {
                Ok(Observed {
                    value:
                        CompareOutcome {
                            swapped: true,
                            current: _written,
                        },
                    warning: None,
                }) => {
                    return Ok(Observed {
                        value: queue::scheduled(&next),
                        warning: None,
                    });
                }
                Ok(Observed {
                    value:
                        CompareOutcome {
                            swapped: false,
                            current,
                        },
                    warning: None,
                }) => expected = current,
                Ok(Observed {
                    value: _refused,
                    warning: Some(warning),
                }) => return Err(SharedChange::Unavailable(warning)),
                Err(_error) => {
                    return Err(SharedChange::Unavailable(unavailable("compare_and_set")));
                }
            }
        }
        Err(SharedChange::Refused(SchedulerError::Contended {
            model: model.to_owned(),
            attempts: MAX_CAS_ATTEMPTS,
        }))
    }
    fn change_local<F>(
        &self,
        key: &RouteKey,
        model: &str,
        change: &F,
        warning: Option<StoreWarning>,
    ) -> Result<Observed<Vec<SchedulerTuple>>, SchedulerError>
    where
        F: Fn(BTreeSet<String>) -> Result<BTreeSet<String>, SchedulerError>,
    {
        let mut expected = self
            .local
            .get(key)
            .map_err(|source| SchedulerError::Local { source })?;
        for _attempt in 0..MAX_CAS_ATTEMPTS {
            let next = change(queue::members(&expected, model)?)?;
            let outcome = self
                .local
                .compare_and_set(
                    key,
                    &expected,
                    &RouteValue::Members(next.clone()),
                    self.local_ttl(),
                )
                .map_err(|source| SchedulerError::Local { source })?;
            if outcome.swapped {
                return Ok(Observed {
                    value: queue::scheduled(&next),
                    warning,
                });
            }
            expected = outcome.current;
        }
        Err(SchedulerError::Contended {
            model: model.to_owned(),
            attempts: MAX_CAS_ATTEMPTS,
        })
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
// the encoded members a queue write stores, round-tripped through the tuple form DualCache would carry
fn encoded(entries: &[SchedulerTuple]) -> RouteValue {
    RouteValue::Members(
        entries
            .iter()
            .map(|entry| queue::encode(entry.priority, &entry.request_id))
            .collect(),
    )
}
