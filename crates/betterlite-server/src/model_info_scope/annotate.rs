use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::teams::TeamModelLists;
use super::text_field;

fn team_eligible(info: &JsonObject, model_id: &str, team_models: &TeamModelLists) -> bool {
    if matches!(info.get("team_id"), None | Some(Json::Null)) {
        return true;
    }
    text_field(info, "team_id").is_some_and(|team| {
        team_models
            .get(model_id)
            .is_some_and(|team_ids| team_ids.iter().any(|id| id == team))
    })
}

pub fn populate_team_access(
    all_models: &mut [DeploymentSpec],
    team_models: Option<&TeamModelLists>,
    direct_ids: &[String],
) {
    if let Some(mapped) = team_models {
        for model in all_models.iter_mut() {
            let Some(model_id) = model.id().map(str::to_owned) else {
                continue;
            };
            if !team_eligible(&model.model_info, &model_id, mapped) {
                continue;
            }
            let team_ids = mapped.get(&model_id).cloned().unwrap_or_default();
            model.model_info.insert(
                "access_via_team_ids".to_owned(),
                Json::List(team_ids.into_iter().map(Json::Text).collect()),
            );
        }
    }
    let direct: BTreeSet<&str> = direct_ids.iter().map(String::as_str).collect();
    for model in all_models.iter_mut() {
        let Some(model_id) = model.id().map(str::to_owned) else {
            continue;
        };
        model.model_info.insert(
            "direct_access".to_owned(),
            Json::Bool(direct.contains(model_id.as_str())),
        );
    }
}

// The annotations this policy writes are booleans and lists, so only those two shapes are visible.
#[must_use]
pub fn visible_models(all_models: Vec<DeploymentSpec>) -> Vec<DeploymentSpec> {
    all_models
        .into_iter()
        .filter(|model| {
            let direct = matches!(
                model.model_info.get("direct_access"),
                Some(Json::Bool(true))
            );
            let team_ids = matches!(
                model.model_info.get("access_via_team_ids"),
                Some(Json::List(ids)) if !ids.is_empty()
            );
            direct || team_ids
        })
        .collect()
}
