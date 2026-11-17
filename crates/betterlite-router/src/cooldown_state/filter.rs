use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;

// the router settings the health filter reads: the opt-in flag, the policy exemption and the group allowlist
#[derive(Clone, Copy, Debug)]
pub struct HealthRouting<'settings> {
    pub enabled: bool,
    pub has_allowed_fails_policy: bool,
    pub scoped_groups: Option<&'settings BTreeSet<String>>,
}

// the health routing settings and the snapshot's verdicts, as one route service answers them
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HealthInputs {
    pub enabled: bool,
    pub has_allowed_fails_policy: bool,
    pub scoped_groups: Option<BTreeSet<String>>,
    pub unhealthy: BTreeSet<String>,
}

// _filter_cooldown_deployments: a deployment that is cooling down right now is not a candidate
#[must_use]
pub fn without_cooling_down(
    deployments: Vec<DeploymentSpec>,
    cooldown_ids: &[String],
) -> Vec<DeploymentSpec> {
    let cooling: BTreeSet<&str> = cooldown_ids.iter().map(String::as_str).collect();
    deployments
        .into_iter()
        .filter(|deployment| deployment.id().is_none_or(|id| !cooling.contains(id)))
        .collect()
}

// _async_filter_health_check_unhealthy_deployments: drop the deployments an active probe marked unhealthy
#[must_use]
pub fn without_unhealthy(
    deployments: Vec<DeploymentSpec>,
    unhealthy: &BTreeSet<String>,
    routing: HealthRouting<'_>,
    health_check_probe: bool,
) -> Vec<DeploymentSpec> {
    if !routing.enabled || (routing.has_allowed_fails_policy && routing.scoped_groups.is_none()) {
        return deployments;
    }
    if unhealthy.is_empty() {
        return deployments;
    }
    let kept: Vec<DeploymentSpec> = deployments
        .iter()
        .filter(|deployment| {
            let marked = deployment.id().is_some_and(|id| unhealthy.contains(id));
            let scoped_out = routing
                .scoped_groups
                .is_some_and(|groups| !groups.contains(&deployment.model_name));
            !marked || scoped_out
        })
        .cloned()
        .collect();
    // the safety net: routing keeps its candidates when every one of them is unhealthy, unless the caller is a health probe
    if kept.is_empty() {
        if health_check_probe {
            return Vec::new();
        }
        tracing::warn!(
            "all deployments marked unhealthy by health checks, bypassing health filter"
        );
        return deployments;
    }
    kept
}

// _is_deployment_blocked: an admin-paused deployment never serves a request, whichever route asks
#[must_use]
pub fn is_blocked(deployment: &DeploymentSpec) -> bool {
    matches!(deployment.model_info.get("blocked"), Some(Json::Bool(true)))
}

// _are_all_deployments_blocked, and _is_model_fully_blocked over one model's deployments
#[must_use]
pub fn all_blocked(deployments: &[DeploymentSpec]) -> bool {
    !deployments.is_empty() && deployments.iter().all(is_blocked)
}

// get_fully_blocked_model_names: the model names whose every deployment an admin paused
#[must_use]
pub fn fully_blocked_model_names(deployments: &[DeploymentSpec]) -> BTreeSet<String> {
    let mut blocked: BTreeMap<&str, bool> = BTreeMap::new();
    for deployment in deployments {
        let name = deployment.model_name.as_str();
        if name.is_empty() {
            continue;
        }
        let paused = is_blocked(deployment);
        blocked
            .entry(name)
            .and_modify(|every| *every = *every && paused)
            .or_insert(paused);
    }
    names_where_every(blocked)
}

// async_get_fully_unhealthy_model_names: the names, team aliases included, whose every deployment a probe marked unhealthy
#[must_use]
pub fn fully_unhealthy_model_names(
    deployments: &[DeploymentSpec],
    unhealthy: &BTreeSet<String>,
    has_allowed_fails_policy: bool,
) -> BTreeSet<String> {
    if has_allowed_fails_policy {
        return BTreeSet::new();
    }
    let mut marked: BTreeMap<&str, bool> = BTreeMap::new();
    for deployment in deployments {
        let mut names = vec![deployment.model_name.as_str()];
        if let Some(Json::Text(team_name)) = deployment.model_info.get("team_public_model_name") {
            names.push(team_name.as_str());
        }
        let verdict = deployment.id().is_some_and(|id| unhealthy.contains(id));
        for name in names {
            if name.is_empty() {
                continue;
            }
            marked
                .entry(name)
                .and_modify(|every| *every = *every && verdict)
                .or_insert(verdict);
        }
    }
    names_where_every(marked)
}

fn names_where_every(verdicts: BTreeMap<&str, bool>) -> BTreeSet<String> {
    verdicts
        .into_iter()
        .filter(|(_name, every)| *every)
        .map(|(name, _every)| name.to_owned())
        .collect()
}
