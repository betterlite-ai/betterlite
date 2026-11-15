use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::deployment_clients::Settings;
use betterlite_router::deployment_clients::clients::CachedClients;
use betterlite_router::deployment_clients::guard::{LimitSource, MaxParallelRequestsLimit};
use serde::{Deserialize, Serialize};

use crate::router_probes::error::OperationError;

const MAX_LOOKUPS: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedInput {
    pub model_id: String,
    pub model_group: String,
    pub max_parallel_requests: Option<i64>,
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
    pub default_max_parallel_requests: Option<i64>,
    pub lookups: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedObservation {
    pub installed: bool,
    pub limit: CachedLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedLimit {
    Absent,
    New,
    Reused,
}

pub(crate) fn cached_outcome(
    input: &CachedInput,
) -> Result<Vec<CachedObservation>, OperationError> {
    if input.lookups > MAX_LOOKUPS {
        return Err(OperationError::InvalidRequest {
            message: format!("lookups must be at most {MAX_LOOKUPS}"),
        });
    }
    let clients = CachedClients::new(
        Duration::from_secs(Settings::default().client_ttl_seconds),
        Arc::new(|| UNIX_EPOCH),
    );
    let source = LimitSource {
        max_parallel_requests: input.max_parallel_requests,
        rpm: input.rpm,
        tpm: input.tpm,
        default_max_parallel_requests: input.default_max_parallel_requests,
    };
    let mut previous: Option<Arc<MaxParallelRequestsLimit>> = None;
    let mut observations = Vec::new();
    for _ in 0..input.lookups {
        let lookup = clients.max_parallel_requests(&input.model_id, &input.model_group, source);
        let limit = match (previous.as_ref(), lookup.limit.as_ref()) {
            (_, None) => CachedLimit::Absent,
            (Some(before), Some(now)) if Arc::ptr_eq(before, now) => CachedLimit::Reused,
            _ => CachedLimit::New,
        };
        previous = lookup.limit;
        observations.push(CachedObservation {
            installed: lookup.initialized,
            limit,
        });
    }
    Ok(observations)
}
