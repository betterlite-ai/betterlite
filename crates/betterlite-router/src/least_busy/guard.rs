use std::sync::Arc;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, RoutingValueStore};

use crate::routing_state_store::error::Observed;

use super::error::{LeastBusyError, LeastBusyWarning, UnwrittenCount};
use super::identity::{AttemptFacts, deployment_ref, request_count_key};
use super::{IN_FLIGHT_COUNT_TTL_SECONDS, LeastBusy};

// what one pre-call callback started: the release it owes, or none when the attempt names no counter
pub struct Started {
    pub call: Option<InFlightCall>,
    pub warning: Option<LeastBusyWarning>,
}

// the ownership guard: success, mapped failure or a cancelled stream consumes it, and a dropped one releases this worker's count
pub struct InFlightCall {
    service: Arc<LeastBusy>,
    key: RouteKey,
    released: bool,
}

// whether this worker's counter took the write, and the warning a rejected one raised
enum LocalWrite {
    Written,
    Rejected(LeastBusyWarning),
}

impl LeastBusy {
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
            call: Some(InFlightCall {
                service: Arc::clone(self),
                key,
                released: false,
            }),
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
            && value < 0
        {
            let _reset = self.local.set(key, &RouteValue::Counter(0), ttl);
        }
    }
}

// increment_cache with local_only: a counter driven below zero is written back as zero
fn write_local(
    store: &RoutingValueStore,
    key: &RouteKey,
    delta: i64,
    ttl: Option<f64>,
) -> LocalWrite {
    match store.increment(key, &RouteIncrement::Counter(delta), ttl) {
        Ok(RouteValue::Counter(value)) if value < 0 => {
            match store.set(key, &RouteValue::Counter(0), ttl) {
                Ok(()) => LocalWrite::Written,
                Err(source) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::Local(source))),
            }
        }
        Ok(RouteValue::Counter(_)) => LocalWrite::Written,
        Ok(_) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::NotACount)),
        Err(source) => LocalWrite::Rejected(unwritable(key, UnwrittenCount::Local(source))),
    }
}

// increment_with_floor: the corrective write that clamps a negative shared count is the second half of the floor
async fn write_shared(
    store: &RoutingStateStore,
    key: &RouteKey,
    delta: i64,
    ttl: Option<f64>,
) -> Option<LeastBusyWarning> {
    let failed = match store
        .increment(key, &RouteIncrement::Counter(delta), ttl)
        .await
    {
        Ok(Observed { value, .. }) => match value {
            RouteValue::Counter(value) if value < 0 => {
                let floor = value.saturating_neg();
                store
                    .increment(key, &RouteIncrement::Counter(floor), ttl)
                    .await
                    .err()
                    .map(UnwrittenCount::Shared)
            }
            RouteValue::Counter(_) => None,
            _ => Some(UnwrittenCount::NotACount),
        },
        Err(source) => Some(UnwrittenCount::Shared(source)),
    };
    failed.map(|cause| unwritable(key, cause))
}

fn unwritable(key: &RouteKey, cause: UnwrittenCount) -> LeastBusyWarning {
    LeastBusyWarning::Unwritable {
        key: key.as_ref().to_owned(),
        cause,
    }
}

impl InFlightCall {
    // log_success_event, log_failure_event and the closed stream all release the one call they own
    pub async fn release(mut self) -> Option<LeastBusyWarning> {
        self.released = true;
        self.service.increment(&self.key, -1).await
    }
}

impl Drop for InFlightCall {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.service.release_locally(&self.key);
    }
}
