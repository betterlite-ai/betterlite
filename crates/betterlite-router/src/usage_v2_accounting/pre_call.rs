use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::router_observations::counter::CounterSink as _;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::DeltaStore;

use super::deployment::DeploymentFacts;
use super::error::RpmLimitExceeded;
use super::keys::window_keys;

// the fields a rejection carries: the async check sends a retry header and the deployment's
// num_retries, the sync check sends neither
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rejection {
    retry_after_seconds: Option<u64>,
    num_retries: Option<i64>,
}

impl Rejection {
    // async_pre_call_check passes headers={"retry-after": str(60)} and num_retries=deployment.get("num_retries")
    pub(super) const fn async_for(deployment: &DeploymentFacts<'_>) -> Self {
        Self {
            retry_after_seconds: Some(RpmLimitExceeded::RETRY_AFTER_SECONDS),
            num_retries: deployment.num_retries,
        }
    }

    // every field litellm.RateLimitError receives for a deployment over its own rpm limit
    fn error(
        self,
        deployment: &DeploymentFacts<'_>,
        limit: i64,
        usage: Decimal,
    ) -> RpmLimitExceeded {
        RpmLimitExceeded {
            limit,
            usage,
            model: deployment.model.to_owned(),
            num_retries: self.num_retries,
            retry_after_seconds: self.retry_after_seconds,
        }
    }
}

// the window key a request counts against and the rpm limit its deployment selected
pub(super) struct Reservation {
    pub(super) key: String,
    pub(super) limit: Option<i64>,
}

// LiteLLM rejects a count already at the limit before it counts the request
pub(super) fn reserve<S>(
    counters: &RouterCounters<S>,
    deployment: &DeploymentFacts<'_>,
    now: DateTime<Utc>,
    rejection: &Rejection,
) -> Result<Reservation, RpmLimitExceeded>
where
    S: DeltaStore + 'static,
{
    let key = window_keys(deployment.model_id, deployment.model, now).rpm;
    let selected = deployment
        .limits
        .rpm
        .or(deployment.limits.params_rpm)
        .or(deployment.limits.model_info_rpm);
    let Some(limit) = selected else {
        return Ok(Reservation { key, limit: None });
    };
    let Some(current) = counters.read(&key) else {
        return Ok(Reservation {
            key,
            limit: selected,
        });
    };
    if current >= limit {
        return Err(rejection.error(deployment, limit, Decimal::from(current)));
    }
    Ok(Reservation {
        key,
        limit: selected,
    })
}

// LiteLLM counts the request first and only then rejects a count past the boundary
pub(super) fn past_limit(
    deployment: &DeploymentFacts<'_>,
    limit: Option<i64>,
    updated: Decimal,
    rejection: &Rejection,
) -> Result<(), RpmLimitExceeded> {
    match limit {
        Some(limit) if updated > Decimal::from(limit) => {
            Err(rejection.error(deployment, limit, updated))
        }
        Some(_) | None => Ok(()),
    }
}
