pub mod error;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;

use self::error::RouterHealthError;
use crate::router_dependencies::dependencies::strategy_router_dependencies;
use crate::router_dependencies::{
    StrategyRouterDependency, StrategyRouterDependencyRole, classify_strategy_router_model,
};

mod tests;

// One probed deployment as the health run reports it: the id it was filed under and, when the probe failed, why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub model_id: Option<String>,
    pub error: Option<String>,
}

// _deployment_id: model_info.id as text, and an id that is absent, empty or not text names no deployment.
#[must_use]
pub fn deployment_id(deployment: &DeploymentSpec) -> Option<&str> {
    let id = deployment.id()?;
    if id.is_empty() {
        return None;
    }
    Some(id)
}

// _resolved_deployment_ids: the ids of every deployment a request for model_name routes to, or none when it routes nowhere.
pub fn resolved_deployment_ids(
    lookup: &CatalogLookup<'_>,
    model_name: &str,
) -> Result<Option<BTreeSet<String>>, RouterHealthError> {
    let resolved = lookup.get_model_list(Some(model_name), None)?;
    if resolved.is_empty() {
        return Ok(None);
    }
    let ids: BTreeSet<String> = resolved
        .iter()
        .filter_map(deployment_id)
        .map(str::to_owned)
        .collect();
    Ok(Some(ids))
}

// _deployments_by_id and filter_deployments_by_id: the universe rows for ids, one per id, in universe order.
fn deployments_by_id(universe: &[DeploymentSpec], ids: &BTreeSet<String>) -> Vec<DeploymentSpec> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut kept = Vec::new();
    for deployment in universe {
        let Some(id) = deployment_id(deployment) else {
            continue;
        };
        if ids.contains(id) && seen.insert(id) {
            kept.push(deployment.clone());
        }
    }
    kept
}

// ast-grep-ignore: no-long-names - LiteLLM's _dependency_deployments_to_probe, kept for parity
pub fn dependency_deployments_to_probe(
    checked: &[DeploymentSpec],
    universe: &[DeploymentSpec],
    lookup: &CatalogLookup<'_>,
) -> Result<Vec<DeploymentSpec>, RouterHealthError> {
    let checked_ids: BTreeSet<String> = checked
        .iter()
        .filter_map(deployment_id)
        .map(str::to_owned)
        .collect();
    let mut reached = checked_ids.clone();
    let mut frontier = checked.to_vec();
    for _ in 0..universe.len() {
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for deployment in &frontier {
            for dependency in strategy_router_dependencies(&deployment.litellm_params) {
                if dependency.role != StrategyRouterDependencyRole::Evaluation {
                    names.insert(dependency.model_name.as_str());
                }
            }
        }
        let mut fresh: BTreeSet<String> = BTreeSet::new();
        for name in names {
            let Some(resolved) = resolved_deployment_ids(lookup, name)? else {
                continue;
            };
            fresh.extend(resolved.into_iter().filter(|id| !reached.contains(id)));
        }
        if fresh.is_empty() {
            break;
        }
        frontier = deployments_by_id(universe, &fresh);
        reached.extend(fresh);
    }
    let dependency_ids: BTreeSet<String> = reached.difference(&checked_ids).cloned().collect();
    Ok(deployments_by_id(universe, &dependency_ids))
}

// _dependency_failure: why this dependency leaves its router unable to serve, when it does.
fn dependency_failure(
    dependency: &StrategyRouterDependency,
    lookup: &CatalogLookup<'_>,
    unhealthy_ids: &BTreeSet<String>,
) -> Result<Option<String>, RouterHealthError> {
    let role = dependency.role.name();
    let name = &dependency.model_name;
    let Some(resolved) = resolved_deployment_ids(lookup, name)? else {
        return Ok(Some(format!(
            "{role} model '{name}' matches no deployment on this proxy"
        )));
    };
    if resolved.is_empty() || !resolved.is_subset(unhealthy_ids) {
        return Ok(None);
    }
    Ok(Some(format!(
        "{role} model '{name}' has no healthy deployment"
    )))
}

// _strategy_router_dependency_error: the first dependency fault that leaves the router unable to serve, if any.
fn first_dependency_fault(
    deployment: &DeploymentSpec,
    lookup: &CatalogLookup<'_>,
    unhealthy_ids: &BTreeSet<String>,
) -> Result<Option<String>, RouterHealthError> {
    for dependency in strategy_router_dependencies(&deployment.litellm_params) {
        if dependency.role == StrategyRouterDependencyRole::Evaluation {
            continue;
        }
        if let Some(failure) = dependency_failure(&dependency, lookup, unhealthy_ids)? {
            return Ok(Some(failure));
        }
    }
    Ok(None)
}

// _is_strategy_router_deployment: litellm_params.model selects one of the strategy routers.
fn is_marker(deployment: &DeploymentSpec) -> bool {
    matches!(
        deployment.litellm_params.get("model"),
        Some(Json::Text(model)) if classify_strategy_router_model(model).is_some()
    )
}

// The routers a healthy probe reported, keyed by id: only these can gain a verdict, since a marker is never probed itself.
fn marked_routers<'checked>(
    healthy_endpoints: &[Endpoint],
    checked: &'checked [DeploymentSpec],
) -> BTreeMap<String, &'checked DeploymentSpec> {
    let by_id: BTreeMap<&str, &DeploymentSpec> = checked
        .iter()
        .filter_map(|deployment| Some((deployment_id(deployment)?, deployment)))
        .collect();
    let mut marked = BTreeMap::new();
    for endpoint in healthy_endpoints {
        let Some(id) = endpoint.model_id.as_deref() else {
            continue;
        };
        let Some(&deployment) = by_id.get(id) else {
            continue;
        };
        if is_marker(deployment) {
            marked.insert(id.to_owned(), deployment);
        }
    }
    marked
}

// The ids this run judged unhealthy, which a dependency must cover entirely to red its router.
fn probe_failures(unhealthy_endpoints: &[Endpoint]) -> BTreeSet<String> {
    unhealthy_endpoints
        .iter()
        .filter_map(|endpoint| endpoint.model_id.as_deref())
        .map(str::to_owned)
        .collect()
}

// Verdicts settle round by round, each feeding the last round's reds back in as unhealthy, bounded by the marker count so a router cycle stops.
fn settle_verdicts(
    marked: &BTreeMap<String, &DeploymentSpec>,
    lookup: &CatalogLookup<'_>,
    failures: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>, RouterHealthError> {
    let mut settled: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..marked.len() {
        let mut unhealthy = failures.clone();
        unhealthy.extend(settled.keys().cloned());
        let mut fresh: BTreeMap<String, String> = BTreeMap::new();
        for (id, &deployment) in marked {
            if settled.contains_key(id) {
                continue;
            }
            if let Some(error) = first_dependency_fault(deployment, lookup, &unhealthy)? {
                fresh.insert(id.clone(), error);
            }
        }
        if fresh.is_empty() {
            break;
        }
        settled.extend(fresh);
    }
    Ok(settled)
}

// _strategy_router_verdicts: the dependency fault, per model id, for every checked strategy router that cannot serve.
pub fn strategy_router_verdicts(
    healthy_endpoints: &[Endpoint],
    unhealthy_endpoints: &[Endpoint],
    checked: &[DeploymentSpec],
    lookup: &CatalogLookup<'_>,
) -> Result<BTreeMap<String, String>, RouterHealthError> {
    let marked = marked_routers(healthy_endpoints, checked);
    let failures = probe_failures(unhealthy_endpoints);
    settle_verdicts(&marked, lookup, &failures)
}

// keep: an endpoint probed only to reach a router answers a deployment the caller never asked about.
fn kept(endpoint: &Endpoint, dropped: &BTreeSet<&str>) -> bool {
    endpoint
        .model_id
        .as_deref()
        .is_none_or(|id| !dropped.contains(id))
}

// ast-grep-ignore: no-long-names - LiteLLM's _finalize_strategy_router_endpoints, kept for parity
pub fn finalize_strategy_router_endpoints(
    healthy_endpoints: &[Endpoint],
    unhealthy_endpoints: &[Endpoint],
    checked: &[DeploymentSpec],
    lookup: Option<&CatalogLookup<'_>>,
    dependency_probes: &[DeploymentSpec],
) -> Result<(Vec<Endpoint>, Vec<Endpoint>), RouterHealthError> {
    let verdicts = match lookup {
        Some(catalog) => {
            strategy_router_verdicts(healthy_endpoints, unhealthy_endpoints, checked, catalog)?
        }
        None => BTreeMap::new(),
    };
    let dropped: BTreeSet<&str> = dependency_probes.iter().filter_map(deployment_id).collect();
    let mut healthy = Vec::new();
    let mut unhealthy = Vec::new();
    for endpoint in unhealthy_endpoints {
        if kept(endpoint, &dropped) {
            unhealthy.push(endpoint.clone());
        }
    }
    for endpoint in healthy_endpoints {
        if !kept(endpoint, &dropped) {
            continue;
        }
        match endpoint.model_id.as_deref().and_then(|id| verdicts.get(id)) {
            Some(error) => unhealthy.push(Endpoint {
                model_id: endpoint.model_id.clone(),
                error: Some(error.clone()),
            }),
            None => healthy.push(endpoint.clone()),
        }
    }
    Ok((healthy, unhealthy))
}
