use std::convert::Infallible;

use betterlite_models::decisions::Json;
use thiserror::Error;

use super::health::HealthyDeployments;
use super::metadata::{Evidence, Lar1Metadata, Time};
use super::thresholds::{ThresholdArgs, ThresholdError, Thresholds};
use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;

// _classify_request answers the deployment type a request's confidence belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    CloudSmart,
    CloudFast,
    Local,
    Deep,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Lar1Error {
    // get_available_deployment refuses a synchronous caller, and the SDK layer reports this message
    #[error(
        "LAR-1 routing only supports async routing. Enable async_only_mode on the router or use acompletion."
    )]
    AsyncOnly,
}

impl Target {
    // the model_info type LiteLLM's classifier names, which is also the deployment tier
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::CloudSmart => "cloud-smart",
            Self::CloudFast => "cloud-fast",
            Self::Local => "local",
            Self::Deep => "deep",
        }
    }
}

// LiteLLM's get_available_deployment raises before any routing work, so no synchronous port exists
pub fn sync_deployment() -> Result<Infallible, Lar1Error> {
    tracing::debug!("LAR-1 routing only has an async selector");
    Err(Lar1Error::AsyncOnly)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lar1Strategy {
    pub thresholds: Thresholds,
}

impl Lar1Strategy {
    // lar1_thresholds_from_args then _normalize_thresholds: an invalid order fails startup
    pub fn new(args: Option<&ThresholdArgs>) -> Result<Self, ThresholdError> {
        Ok(Self {
            thresholds: Thresholds::from_args(args)?,
        })
    }

    // _classify_request: unverified evidence outranks the time dimension, which outranks confidence
    #[must_use]
    pub fn target(&self, metadata: &Lar1Metadata) -> Target {
        if metadata.evidence.contains(&Evidence::Unverified) {
            return Target::CloudSmart;
        }
        if metadata.time == Time::Mem {
            return Target::CloudFast;
        }
        let confidence = metadata.confidence;
        if confidence < self.thresholds.low {
            Target::CloudSmart
        } else if confidence < self.thresholds.medium {
            Target::CloudFast
        } else if confidence < self.thresholds.high {
            Target::Local
        } else {
            Target::Deep
        }
    }

    // async_get_available_deployment: parse the request metadata, then select over the healthy pool
    pub async fn select_deployment(
        &self,
        router: Option<&dyn HealthyDeployments>,
        metadata: Option<&[u8]>,
        specific_deployment: bool,
        model: &str,
    ) -> Result<Option<DeploymentSpec>, RoutingError> {
        let Some(pool) = router else {
            return Ok(None);
        };
        let lar1 = metadata.map_or_else(Lar1Metadata::default, Lar1Metadata::from_request_metadata);
        let deployments = match pool.healthy_deployments(model, specific_deployment).await? {
            Candidates::Single(deployment) => return Ok(Some(*deployment)),
            Candidates::Pool(deployments) => deployments,
        };
        let target = self.target(&lar1);
        let Some((index, exact)) = pick(&deployments, target) else {
            return Ok(None);
        };
        if exact {
            tracing::info!(
                confidence = %lar1.confidence,
                classified = target.wire_name(),
                "LAR-1 confidence routing"
            );
        } else {
            let actual = deployments.get(index).and_then(kind).unwrap_or("unknown");
            tracing::warn!(
                classified = target.wire_name(),
                actual,
                "LAR-1 has no deployment for the classified type and falls back"
            );
        }
        Ok(deployments.get(index).cloned())
    }
}

// _select_deployment: the first deployment of the target type, else the first healthy one
#[must_use]
pub fn pick(deployments: &[DeploymentSpec], target: Target) -> Option<(usize, bool)> {
    if deployments.is_empty() {
        return None;
    }
    match deployments
        .iter()
        .position(|deployment| kind(deployment) == Some(target.wire_name()))
    {
        Some(index) => Some((index, true)),
        None => Some((0, false)),
    }
}

fn kind(deployment: &DeploymentSpec) -> Option<&str> {
    match deployment.model_info.get("type") {
        Some(Json::Text(kind)) => Some(kind),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
