use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;
use crate::usage_routing::window::{Counts, UsageWindows, window_keys};

// The two minute maps get_available_deployments reads; a map the cache does not hold is the cold window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowCounts<'counts> {
    pub tpm: Option<&'counts Counts>,
    pub rpm: Option<&'counts Counts>,
}

// get_available_deployments: the lowest absolute tpm among the deployments no limit rules out, first seen on a tie.
pub fn select_counts(
    counts: &WindowCounts<'_>,
    deployments: &[DeploymentSpec],
    input_tokens: i64,
) -> Result<Option<usize>, RoutingError> {
    let mut candidates = counts.tpm.cloned().unwrap_or_default();
    for id in deployments.iter().filter_map(DeploymentSpec::id) {
        candidates.entry(id.to_owned()).or_insert(0);
    }
    let mut lowest: Option<i64> = None;
    let mut selected: Option<usize> = None;
    for (id, used) in &candidates {
        let Some((index, deployment)) = deployments
            .iter()
            .enumerate()
            .rev()
            .find(|(_, deployment)| deployment.id() == Some(id.as_str()))
        else {
            continue;
        };
        let token_limit = limit(deployment, "tpm")?;
        let request_limit = limit(deployment, "rpm")?;
        let requests_used = counts.rpm.and_then(|stored| stored.get(id)).copied();
        let tokens_exceeded = i128::from(*used).saturating_add(i128::from(input_tokens))
            > token_limit.unwrap_or(i128::MAX);
        let requests_exceeded = requests_used.is_some_and(|count| {
            i128::from(count).saturating_add(1) >= request_limit.unwrap_or(i128::MAX)
        });
        if tokens_exceeded || requests_exceeded {
            continue;
        }
        if lowest.is_none_or(|low| *used < low) {
            lowest = Some(*used);
            selected = Some(index);
        }
    }
    Ok(selected)
}

// get_available_deployments with its two cache reads; a read the store fails is the cold window.
pub fn select(
    windows: &dyn UsageWindows,
    group: &str,
    deployments: &[DeploymentSpec],
    input_tokens: i64,
    now: DateTime<Utc>,
) -> Result<Option<usize>, RoutingError> {
    let (tokens_key, requests_key) = window_keys(group, now);
    let stored_tokens = windows.read(&tokens_key).ok().flatten();
    let stored_requests = windows.read(&requests_key).ok().flatten();
    let counts = WindowCounts {
        tpm: stored_tokens.as_ref(),
        rpm: stored_requests.as_ref(),
    };
    select_counts(&counts, deployments, input_tokens)
}

// _deployment.get("tpm"), then litellm_params.tpm, then model_info.tpm; a null or absent limit is unbounded.
fn limit(deployment: &DeploymentSpec, field: &'static str) -> Result<Option<i128>, RoutingError> {
    for source in [
        &deployment.extra,
        &deployment.litellm_params,
        &deployment.model_info,
    ] {
        match source.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Number(number)) => {
                return number
                    .as_i128()
                    .map(Some)
                    .ok_or(RoutingError::Metric { field });
            }
            Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                return Err(RoutingError::Metric { field });
            }
        }
    }
    Ok(None)
}
