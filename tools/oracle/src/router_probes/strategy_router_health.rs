use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_server::health::router::Endpoint;
// ast-grep-ignore: no-long-names - LiteLLM's _dependency_deployments_to_probe, kept for parity
use betterlite_server::health::router::dependency_deployments_to_probe as expansion;
use betterlite_server::health::router::deployment_id;
// ast-grep-ignore: no-long-names - LiteLLM's _finalize_strategy_router_endpoints, kept for parity
use betterlite_server::health::router::finalize_strategy_router_endpoints as finalize;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub litellm_params: JsonObject,
    #[serde(default)]
    pub model_info: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub models: Vec<Entry>,
    #[serde(default)]
    pub model_group_alias: BTreeMap<String, String>,
    #[serde(default)]
    pub model_id: Option<String>,
    // the ids whose probes fail; the Python side reaches the same outcome by handing the deployment a model it cannot probe
    #[serde(default)]
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub model_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub healthy: Vec<String>,
    pub unhealthy: Vec<Row>,
}

fn spec(entry: &Entry) -> Result<DeploymentSpec, OperationError> {
    DeploymentSpec::new(
        entry.model_name.clone(),
        entry.litellm_params.clone(),
        entry.model_info.clone(),
        JsonObject::new(),
        String::new,
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

// _narrow_to_target: the id the caller named, or the whole list when the id matches nothing.
fn narrowed(rows: &[DeploymentSpec], model_id: Option<&str>) -> Vec<DeploymentSpec> {
    let Some(model_id) = model_id else {
        return rows.to_vec();
    };
    let matched: Vec<DeploymentSpec> = rows
        .iter()
        .filter(|row| deployment_id(row) == Some(model_id))
        .cloned()
        .collect();
    if matched.is_empty() {
        return rows.to_vec();
    }
    matched
}

fn listed(endpoint: Endpoint) -> Row {
    Row {
        model_id: endpoint.model_id,
        error: endpoint.error,
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let rows = input
        .models
        .iter()
        .map(spec)
        .collect::<Result<Vec<_>, _>>()?;
    let mut catalog = DeploymentCatalog::default();
    for row in &rows {
        catalog.model_names.insert(row.model_name.clone());
        catalog
            .index_patterns(row)
            .map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        catalog.push(row.clone());
    }
    let mut aliases = AliasMap::new();
    for (alias, target) in &input.model_group_alias {
        aliases.insert(alias.clone(), AliasTarget::Name(target.clone()));
    }
    let groups = RoutingGroupIndex::default();
    let provider = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &provider,
    };
    let requested = narrowed(&rows, input.model_id.as_deref());
    let probes =
        expansion(&requested, &rows, &lookup).map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let mut checked = requested;
    checked.extend(probes.iter().cloned());
    let failed: BTreeSet<&str> = input.failed.iter().map(String::as_str).collect();
    let mut healthy = Vec::new();
    let mut unhealthy = Vec::new();
    for row in &checked {
        let Some(id) = deployment_id(row) else {
            continue;
        };
        if failed.contains(id) {
            unhealthy.push(Endpoint {
                model_id: Some(id.to_owned()),
                error: Some("the probe failed".to_owned()),
            });
        } else {
            healthy.push(Endpoint {
                model_id: Some(id.to_owned()),
                error: None,
            });
        }
    }
    let (graded_healthy, graded_unhealthy) =
        finalize(&healthy, &unhealthy, &checked, Some(&lookup), &probes).map_err(|error| {
            OperationError::InvalidRequest {
                message: error.to_string(),
            }
        })?;
    Ok(ProbeOutput::StrategyRouterHealth(Output {
        healthy: graded_healthy
            .into_iter()
            .filter_map(|endpoint| endpoint.model_id)
            .collect(),
        unhealthy: graded_unhealthy.into_iter().map(listed).collect(),
    }))
}
