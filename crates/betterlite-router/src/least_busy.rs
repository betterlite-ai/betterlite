pub mod error;
pub mod guard;
pub mod identity;

mod tests;

use std::sync::Arc;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StoreWarning};
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use self::error::{LeastBusyError, LeastBusyWarning, UnreadableCounts};
use self::guard::{InFlightCall, LocalWrite, Started, write_local, write_shared};
use self::identity::{AttemptFacts, deployment_ref, request_count_key, request_count_keys};

// IN_FLIGHT_COUNT_TTL_SECONDS: LiteLLM's one hour, counted from the call that created the counter
pub const IN_FLIGHT_COUNT_TTL_SECONDS: f64 = 3600.0;

// the tier a selection read its counts from, and the fallback arm is the one that also logs a warning
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountsSource {
    Shared,
    Local,
    Fallback,
}

// get_available_deployments' answer: a pool index, the tier behind it, and the warning a fallback logged
#[derive(Debug)]
pub struct Selection {
    pub index: Option<usize>,
    pub source: CountsSource,
    pub warning: Option<LeastBusyWarning>,
}

// one read of the shared tier, before the stored values became counts
#[derive(Debug)]
pub enum SharedCounts {
    Absent,
    Values(Vec<Option<i64>>),
    Unreadable(UnreadableCounts),
}

// LeastBusyLoggingHandler: the two tiers the in-flight counts live in, shared absent on a router without Redis
pub struct LeastBusy {
    pub local: RoutingValueStore,
    pub shared: Option<RoutingStateStore>,
}

impl LeastBusy {
    // get_available_deployments and async_get_available_deployments: a shared read that throws falls back to this worker's own counts
    pub async fn select(
        &self,
        model_group: &str,
        deployments: &[DeploymentSpec],
    ) -> Result<Selection, LeastBusyError> {
        // _least_busy answers an empty pool before any tier is read
        if deployments.is_empty() {
            let source = if self.shared.is_some() {
                CountsSource::Shared
            } else {
                CountsSource::Local
            };
            return Ok(Selection {
                index: None,
                source,
                warning: None,
            });
        }
        let keys = request_count_keys(model_group, deployments)?;
        let mut warning = None;
        let (values, source) = match self.shared_counts(&keys).await {
            SharedCounts::Values(values) => (values, CountsSource::Shared),
            SharedCounts::Absent => (self.local_count_values(&keys)?, CountsSource::Local),
            SharedCounts::Unreadable(cause) => {
                warning = Some(LeastBusyWarning::Unreadable {
                    model_group: model_group.to_owned(),
                    cause,
                });
                (self.local_count_values(&keys)?, CountsSource::Fallback)
            }
        };
        // the shared tuple is counts in key order, while a local read of another length is this worker holding nothing
        let counts = match source {
            CountsSource::Shared => as_counts(&values),
            CountsSource::Local | CountsSource::Fallback => {
                local_counts(Some(values.as_slice()), keys.len())
            }
        };
        Ok(Selection {
            index: least_busy(deployments.len(), &counts),
            source,
            warning,
        })
    }

    // log_pre_api_call: one more call in flight under the deployment the attempt picked
    pub async fn pre_call(
        self: &Arc<Self>,
        facts: &AttemptFacts<'_>,
    ) -> Result<Started, LeastBusyError> {
        let Some(reference) = deployment_ref(facts) else {
            return Ok(Started {
                call: None,
                warning: None,
            });
        };
        let key = request_count_key(&reference.model_group, &reference.deployment_id)?;
        let warning = self.increment(&key, 1).await;
        Ok(Started {
            call: Some(InFlightCall::started(Arc::clone(self), key)),
            warning,
        })
    }

    // _increment and _async_increment: this worker's count first, then the shared floor, and a failed write only warns
    async fn increment(&self, key: &RouteKey, delta: i64) -> Option<LeastBusyWarning> {
        let ttl = Some(IN_FLIGHT_COUNT_TTL_SECONDS);
        match write_local(&self.local, key, delta, ttl) {
            LocalWrite::Rejected(warning) => Some(warning),
            LocalWrite::Written => match &self.shared {
                None => None,
                Some(shared) => write_shared(shared, key, delta, ttl).await,
            },
        }
    }

    // a dropped callback cannot await, so the shared count expires with its ttl while this worker's returns at once
    fn release_locally(&self, key: &RouteKey) {
        let ttl = Some(IN_FLIGHT_COUNT_TTL_SECONDS);
        let outcome = self.local.increment(key, &RouteIncrement::Counter(-1), ttl);
        if let Ok(RouteValue::Counter(value)) = outcome
            && value.is_negative()
        {
            let _reset = self.local.set(key, &RouteValue::Counter(0), ttl);
        }
    }

    // the shared tier alone, reported as the store left it so a caller can log or probe it
    #[must_use]
    pub async fn shared_counts(&self, keys: &[RouteKey]) -> SharedCounts {
        let Some(shared) = &self.shared else {
            return SharedCounts::Absent;
        };
        match shared.batch_get(keys).await {
            Ok(Observed {
                warning:
                    Some(StoreWarning::Unavailable {
                        operation: operation_name,
                        ..
                    }),
                ..
            }) => SharedCounts::Unreadable(UnreadableCounts::Store {
                operation: operation_name,
            }),
            Ok(Observed { value, .. }) => match counter_values(keys, value) {
                Ok(values) => SharedCounts::Values(values),
                Err(key) => SharedCounts::Unreadable(UnreadableCounts::NotACount {
                    key: key.as_ref().to_owned(),
                }),
            },
            Err(source) => SharedCounts::Unreadable(UnreadableCounts::Refused(source)),
        }
    }

    // the local tier alone, in pool order, with a key it never wrote left missing
    pub fn local_count_values(
        &self,
        keys: &[RouteKey],
    ) -> Result<Vec<Option<i64>>, LeastBusyError> {
        let reads = self
            .local
            .batch_get(keys)
            .map_err(|source| LeastBusyError::Local { source })?;
        counter_values(keys, reads).map_err(|key| LeastBusyError::NotACount {
            key: key.as_ref().to_owned(),
        })
    }
}

// _least_busy: the first deployment holding the smallest count, and none for an empty pool
#[must_use]
pub fn least_busy(deployments: usize, counts: &[i64]) -> Option<usize> {
    counts
        .iter()
        .take(deployments)
        .enumerate()
        .min_by_key(|(_index, count)| **count)
        .map(|(index, _count)| index)
}

// _as_counts: a counter the store never wrote weighs nothing
#[must_use]
pub fn as_counts(values: &[Option<i64>]) -> Vec<i64> {
    let mut counts = Vec::with_capacity(values.len());
    for value in values {
        counts.push(value.unwrap_or(0));
    }
    counts
}

// _local_counts: a read of another length, or none at all, is this worker holding nothing
#[must_use]
pub fn local_counts(values: Option<&[Option<i64>]>, keys: usize) -> Vec<i64> {
    match values {
        Some(counts) if counts.len() == keys => as_counts(counts),
        None | Some(_) => vec![0; keys],
    }
}

// every stored value in pool order, or the first key whose value is not a counter
pub fn counter_values(
    keys: &[RouteKey],
    reads: Vec<StoreRead>,
) -> Result<Vec<Option<i64>>, RouteKey> {
    let mut values = Vec::with_capacity(reads.len());
    for (key, read) in keys.iter().zip(reads) {
        match read {
            StoreRead::Missing => values.push(None),
            StoreRead::Value(RouteValue::Counter(count)) => values.push(Some(count)),
            StoreRead::Value(_) => return Err(key.clone()),
        }
    }
    Ok(values)
}
