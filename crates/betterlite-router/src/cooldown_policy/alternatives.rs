use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::router_core::candidates::without_blocked;

// routing_group_has_alternatives: the alias target names the group, and the group's members span more than one deployment
#[must_use]
pub fn routing_group_has_alternatives(
    lookup: &CatalogLookup<'_>,
    requested_model_group: Option<&str>,
) -> bool {
    let Some(name) = requested_model_group else {
        return false;
    };
    lookup.routing_groups.has_alternatives(name, &|member| {
        lookup.catalog.name_index.get(member).map_or(0, Vec::len)
    })
}

// team_model_has_alternatives: the deployment's team siblings under the same public model name, blocked ones dropped
#[must_use]
pub fn team_model_has_alternatives(lookup: &CatalogLookup<'_>, deployment_id: &str) -> bool {
    let Ok(Some(deployment)) = lookup.get_deployment(deployment_id) else {
        return false;
    };
    let Some(team_id) = info_text(deployment, "team_id") else {
        return false;
    };
    let Some(public_name) = info_text(deployment, "team_public_model_name") else {
        return false;
    };
    let Some(indices) = lookup
        .catalog
        .team_index
        .get(&(team_id.to_owned(), public_name.to_owned()))
    else {
        return false;
    };
    let siblings: Vec<DeploymentSpec> = indices
        .iter()
        .filter_map(|index| lookup.catalog.models.get(*index).cloned())
        .collect();
    without_blocked(siblings).len() > 1
}

// Deployment.__init__ keeps team_id and team_public_model_name only as text, and the team index skips empty ones
fn info_text<'spec>(spec: &'spec DeploymentSpec, key: &str) -> Option<&'spec str> {
    if let Some(Json::Text(value)) = spec.model_info.get(key) {
        return (!value.is_empty()).then_some(value.as_str());
    }
    None
}
