use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

// ast-grep-ignore: no-long-names - LiteLLM's _finalize_strategy_router_endpoints, kept for parity
use crate::health::router::{Endpoint, finalize_strategy_router_endpoints as finalize_endpoints};
use crate::health_cycle::catalog::HealthCatalog;
use crate::health_cycle::endpoints::{HealthCheckResult, endpoint_id};
use crate::health_cycle::error::HealthCycleError;

fn verdict_input(endpoint: &JsonObject) -> Endpoint {
    Endpoint {
        model_id: endpoint_id(endpoint).map(str::to_owned),
        error: None,
    }
}

fn endpoint_rows(endpoints: &[JsonObject]) -> BTreeMap<String, JsonObject> {
    endpoints
        .iter()
        .filter_map(|endpoint| endpoint_id(endpoint).map(|id| (id.to_owned(), endpoint.clone())))
        .collect()
}

// A moved healthy row shows the dependency verdict as its error; every checked row carries an id, so the lookup finds it
fn project_endpoint(
    endpoint: &Endpoint,
    healthy: &BTreeMap<String, JsonObject>,
    unhealthy: &BTreeMap<String, JsonObject>,
) -> Option<JsonObject> {
    let model_id = endpoint.model_id.as_deref()?;
    if let Some(error) = &endpoint.error {
        let mut moved = healthy.get(model_id)?.clone();
        moved.insert("error".to_owned(), Json::Text(error.clone()));
        return Some(moved);
    }
    healthy
        .get(model_id)
        .or_else(|| unhealthy.get(model_id))
        .cloned()
}

fn project_endpoints(
    graded: &[Endpoint],
    healthy: &BTreeMap<String, JsonObject>,
    unhealthy: &BTreeMap<String, JsonObject>,
) -> Vec<JsonObject> {
    graded
        .iter()
        .filter_map(|endpoint| project_endpoint(endpoint, healthy, unhealthy))
        .collect()
}

// _finalize_strategy_router_endpoints over the display rows: dependency probes leave, a red dependency moves its router to unhealthy
pub(crate) fn rebuild_after_verdicts(
    probed: HealthCheckResult,
    checked: &[DeploymentSpec],
    lookup: Option<&HealthCatalog<'_>>,
    dependency_probes: &[DeploymentSpec],
) -> Result<HealthCheckResult, HealthCycleError> {
    let healthy_input: Vec<Endpoint> = probed.healthy_endpoints.iter().map(verdict_input).collect();
    let unhealthy_input: Vec<Endpoint> = probed
        .unhealthy_endpoints
        .iter()
        .map(verdict_input)
        .collect();
    let (graded_healthy, graded_unhealthy) = match lookup {
        Some(catalog) => catalog.with_lookup(|view| {
            finalize_endpoints(
                &healthy_input,
                &unhealthy_input,
                checked,
                Some(&view),
                dependency_probes,
            )
        })?,
        None => finalize_endpoints(
            &healthy_input,
            &unhealthy_input,
            checked,
            None,
            dependency_probes,
        )?,
    };
    let healthy_rows = endpoint_rows(&probed.healthy_endpoints);
    let unhealthy_rows = endpoint_rows(&probed.unhealthy_endpoints);
    Ok(HealthCheckResult {
        healthy_endpoints: project_endpoints(&graded_healthy, &healthy_rows, &unhealthy_rows),
        unhealthy_endpoints: project_endpoints(&graded_unhealthy, &healthy_rows, &unhealthy_rows),
        exceptions_by_model_id: probed.exceptions_by_model_id,
    })
}
