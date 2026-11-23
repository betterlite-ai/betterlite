use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::teams::TeamModelLists;
use super::text_field;

// Python truthiness, which _filter_models_to_user_accessible applies to whatever JSON a row already carries.
fn truthy(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => !number.as_f64().is_some_and(|amount| amount == 0.0_f64),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
    }
}

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
    if let Some(team_models) = team_models {
        for model in all_models.iter_mut() {
            let Some(model_id) = model.id().map(str::to_owned) else {
                continue;
            };
            if !team_eligible(&model.model_info, &model_id, team_models) {
                continue;
            }
            let team_ids = team_models.get(&model_id).cloned().unwrap_or_default();
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

#[must_use]
pub fn filter_models_to_user_accessible(all_models: Vec<DeploymentSpec>) -> Vec<DeploymentSpec> {
    all_models
        .into_iter()
        .filter(|model| {
            truthy(model.model_info.get("direct_access"))
                || truthy(model.model_info.get("access_via_team_ids"))
        })
        .collect()
}
