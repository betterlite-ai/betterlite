use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_groups::group::RoutingGroup;
use crate::strategy_dispatch::groups::GroupStrategy;

// apply_routing_group_priority: the only group policy a priority group adds is litellm_params.order.
#[must_use]
pub fn prioritized(group: &RoutingGroup, deployment: &DeploymentSpec) -> DeploymentSpec {
    if !matches!(group.routing_strategy, GroupStrategy::Priority) {
        return deployment.clone();
    }
    let Some(priorities) = &group.model_priorities else {
        return deployment.clone();
    };
    let Some(priority) = priorities.get(&deployment.model_name) else {
        return deployment.clone();
    };
    let mut prioritized = deployment.clone();
    prioritized
        .litellm_params
        .insert("order".to_owned(), Json::Number(Number::from(*priority)));
    prioritized
}

// _as_routing_group_row: a member re-emitted under its group's name never carries the member's access groups.
#[must_use]
pub fn group_row(group_name: &str, deployment: &DeploymentSpec) -> DeploymentSpec {
    let mut row = deployment.clone();
    row.model_name = group_name.to_owned();
    let _ = row.model_info.shift_remove("access_groups");
    row
}
