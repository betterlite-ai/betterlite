use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::health_access_and_connections::visibility::HealthRouter;
use crate::model_info_scope::text_field;
use crate::team_access::caller::TeamCaller;

use super::error::ScopeError;
use super::reads::TeamScopeReads;
use super::team_models::{
    authorize_team_id_query, gather_accessible_model_ids, load_team_for_model_filter,
};

fn model_info_of(row: &JsonObject) -> Option<&JsonObject> {
    match row.get("model_info") {
        Some(Json::Object(info)) => Some(info),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn access_via_team(info: &JsonObject, team_id: &str) -> bool {
    match info.get("access_via_team_ids") {
        Some(Json::List(items)) => items.iter().any(|item| match item {
            Json::Text(value) => value == team_id,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::List(_) => false,
        }),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            false
        }
    }
}

pub async fn filter_models_by_team_id<Reads>(
    all_models: &[JsonObject],
    team_id: &str,
    reads: &Reads,
    router: Option<&dyn HealthRouter>,
    caller: Option<&TeamCaller>,
) -> Result<Vec<JsonObject>, ScopeError>
where
    Reads: TeamScopeReads + ?Sized,
{
    if let Some(requester) = caller {
        authorize_team_id_query(team_id, requester, reads).await?;
    }
    let Some(team_object) = load_team_for_model_filter(team_id, reads).await else {
        return Ok(Vec::new());
    };
    let accessible = gather_accessible_model_ids(&team_object, team_id, reads, router).await?;
    let mut filtered = Vec::new();
    for model in all_models {
        let Some(info) = model_info_of(model) else {
            continue;
        };
        if text_field(info, "team_id") == Some(team_id) {
            filtered.push(model.clone());
            continue;
        }
        if access_via_team(info, team_id) {
            filtered.push(model.clone());
            continue;
        }
        if let Some(model_id) = text_field(info, "id").filter(|id| !id.is_empty())
            && accessible.contains(model_id)
        {
            filtered.push(model.clone());
        }
    }
    Ok(filtered)
}
