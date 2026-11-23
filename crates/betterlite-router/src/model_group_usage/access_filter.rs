use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_lookups::access_groups::AccessGroups;
use crate::model_group_lookups::error::AccessGroupError;

// The model grants one UserAPIKeyAuth object exposes to the filter: models and team_models, as declared.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyGrants {
    pub models: Vec<String>,
    pub team_models: Vec<String>,
}

// Router._filter_deployments_by_model_access_groups' inputs, with the principal read out of the two metadata buckets.
pub struct AccessFilterRequest<'request> {
    pub model: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub metadata: Option<&'request KeyGrants>,
    pub litellm_metadata: Option<&'request KeyGrants>,
    pub team_id: Option<&'request str>,
}

// Restrict the candidates to the access groups the caller holds, and hand every unconstrained case back untouched.
pub fn filter_deployments_by_model_access_groups(
    request: AccessFilterRequest<'_>,
    access_groups: &AccessGroups,
    lookup: &CatalogLookup<'_>,
) -> Result<Vec<DeploymentSpec>, AccessGroupError> {
    let AccessFilterRequest {
        model,
        deployments,
        metadata,
        litellm_metadata,
        team_id,
    } = request;
    let Some(allowed) = constrained_grants(model, deployments, metadata, litellm_metadata) else {
        return Ok(deployments.to_vec());
    };
    let groups = access_groups.get_model_access_groups(lookup, Some(model), None, team_id)?;
    let allowed_groups: BTreeSet<&str> = groups
        .keys()
        .map(String::as_str)
        .filter(|group| allowed.contains(group))
        .collect();
    if allowed_groups.is_empty() {
        return Ok(deployments.to_vec());
    }
    let mut filtered = Vec::new();
    for deployment in deployments {
        if listed_in_allowed_group(deployment, &allowed_groups)? {
            filtered.push(deployment.clone());
        }
    }
    Ok(filtered)
}

// The grants that restrict candidates, or none when the model, wildcard or all-proxy grant, or an empty set, bypasses the filter.
fn constrained_grants<'request>(
    model: &str,
    deployments: &[DeploymentSpec],
    metadata: Option<&'request KeyGrants>,
    litellm_metadata: Option<&'request KeyGrants>,
) -> Option<BTreeSet<&'request str>> {
    if deployments.is_empty() {
        return None;
    }
    let principal = metadata.or(litellm_metadata)?;
    let allowed: BTreeSet<&str> = principal
        .models
        .iter()
        .chain(&principal.team_models)
        .map(String::as_str)
        .collect();
    if allowed.is_empty()
        || allowed.contains(model)
        || allowed.contains("*")
        || allowed.contains("all-proxy-models")
    {
        return None;
    }
    Some(allowed)
}

// The candidate's model_info.access_groups intersection, with a value the declared List[str] cannot hold rejected.
fn listed_in_allowed_group(
    deployment: &DeploymentSpec,
    allowed_groups: &BTreeSet<&str>,
) -> Result<bool, AccessGroupError> {
    let declared = match deployment.model_info.get("access_groups") {
        None | Some(Json::Null) => return Ok(false),
        Some(Json::List(items)) => items,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(invalid_groups(deployment));
        }
    };
    for item in declared {
        let Json::Text(group) = item else {
            return Err(invalid_groups(deployment));
        };
        if allowed_groups.contains(group.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn invalid_groups(deployment: &DeploymentSpec) -> AccessGroupError {
    AccessGroupError::InvalidAccessGroups {
        model_name: deployment.model_name.clone(),
    }
}
