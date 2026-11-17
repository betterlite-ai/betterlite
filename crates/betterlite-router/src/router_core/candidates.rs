use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;
use crate::simple_shuffle::Candidate;
use crate::weighted_failover::drop_excluded;

// The deployments one route may still land on, in the order the catalog gave them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CandidateSet {
    pub deployments: Vec<DeploymentSpec>,
}

impl CandidateSet {
    // the _excluded_deployment_ids and _retry_skipped_deployment_ids filter B8440 owns
    #[must_use]
    pub fn excluded(self, ids: &[String]) -> Self {
        Self {
            deployments: drop_excluded(self.deployments, ids, DeploymentSpec::id),
        }
    }
}

// _filter_blocked_deployments: a deployment an admin paused never serves a request, in a pool or alone.
#[must_use]
pub fn without_blocked(deployments: Vec<DeploymentSpec>) -> Vec<DeploymentSpec> {
    deployments
        .into_iter()
        .filter(|deployment| {
            !matches!(deployment.model_info.get("blocked"), Some(Json::Bool(true)))
        })
        .collect()
}

// The deployment ids a no-deployment error reports, in catalog order.
#[must_use]
pub fn deployment_ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect()
}

// The weights B008's simple-shuffle weighs a pool by, read the way router_strategy/simple_shuffle.py reads them.
pub fn weights(deployments: &[DeploymentSpec]) -> Result<Vec<Candidate>, RoutingError> {
    deployments
        .iter()
        .map(|deployment| {
            Ok(Candidate {
                weight: metric(deployment, "weight")?,
                rpm: metric(deployment, "rpm")?,
                tpm: metric(deployment, "tpm")?,
            })
        })
        .collect()
}

// _metric_weight: an absent or null value weighs nothing, and anything but a whole number names its field
fn metric(deployment: &DeploymentSpec, key: &'static str) -> Result<u32, RoutingError> {
    match deployment.litellm_params.get(key) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(RoutingError::Metric { field: key }),
        Some(Json::Text(_) | Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(RoutingError::Metric { field: key })
        }
    }
}
