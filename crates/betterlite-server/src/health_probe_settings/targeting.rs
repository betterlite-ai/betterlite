use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

fn info_text<'spec>(spec: &'spec DeploymentSpec, key: &str) -> Option<&'spec str> {
    match spec.model_info.get(key) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}

fn matching<K>(model_list: &[DeploymentSpec], keep: K) -> Vec<DeploymentSpec>
where
    K: Fn(&DeploymentSpec) -> bool,
{
    model_list
        .iter()
        .filter(|spec| keep(spec))
        .cloned()
        .collect()
}

#[must_use]
pub fn filter_to_model_groups(
    model_list: &[DeploymentSpec],
    model_groups: Option<&BTreeSet<String>>,
) -> Vec<DeploymentSpec> {
    model_groups.map_or_else(
        || model_list.to_vec(),
        |groups| matching(model_list, |spec| groups.contains(&spec.model_name)),
    )
}

#[must_use]
pub fn filter_deployments_by_id(model_list: &[DeploymentSpec]) -> Vec<DeploymentSpec> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut kept = Vec::new();
    for spec in model_list {
        let Some(id) = spec.id().filter(|id| !id.is_empty()) else {
            continue;
        };
        if seen.insert(id) {
            kept.push(spec.clone());
        }
    }
    kept
}

#[must_use]
pub fn health_check_eligible(
    model_list: &[DeploymentSpec],
    skip_disabled: bool,
) -> Vec<DeploymentSpec> {
    matching(model_list, |spec| {
        !(skip_disabled
            && matches!(
                spec.model_info.get("disable_background_health_check"),
                Some(Json::Bool(true))
            ))
    })
}

fn routed_own_copies(
    model_list: &[DeploymentSpec],
    model_name: &str,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    let Some(team) = team_id else {
        return Vec::new();
    };
    matching(model_list, |spec| {
        info_text(spec, "team_id") == Some(team)
            && info_text(spec, "team_public_model_name") == Some(model_name)
    })
}

// _deployments_routed_by_name: a team's own copies, then model_name, then a public name for a caller with no team
fn routed_by_name(
    model_list: &[DeploymentSpec],
    model_name: &str,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    let own_copies = routed_own_copies(model_list, model_name, team_id);
    if !own_copies.is_empty() {
        return own_copies;
    }
    let by_name = matching(model_list, |spec| spec.model_name == model_name);
    if !by_name.is_empty() || team_id.is_some() {
        return by_name;
    }
    matching(model_list, |spec| {
        info_text(spec, "team_public_model_name") == Some(model_name)
    })
}

#[must_use]
pub fn deployments_targeted_by_name(
    model_list: &[DeploymentSpec],
    model: &str,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    let routed = routed_by_name(model_list, model, team_id);
    if !routed.is_empty() {
        return routed;
    }
    matching(model_list, |spec| match spec.litellm_params.get("model") {
        Some(Json::Text(deployed)) => deployed == model,
        _ => false,
    })
}

#[must_use]
pub fn narrow_to_target(
    model_list: &[DeploymentSpec],
    model: Option<&str>,
    model_id: Option<&str>,
    team_id: Option<&str>,
) -> Vec<DeploymentSpec> {
    if let Some(requested_id) = model_id {
        let by_id = matching(model_list, |spec| spec.id() == Some(requested_id));
        return if by_id.is_empty() {
            model_list.to_vec()
        } else {
            by_id
        };
    }
    model.map_or_else(
        || model_list.to_vec(),
        |requested| deployments_targeted_by_name(model_list, requested, team_id),
    )
}
