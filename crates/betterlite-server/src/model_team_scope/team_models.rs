use std::collections::BTreeSet;

use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;

use crate::health_access_and_connections::visibility::HealthRouter;
use crate::model_info_scope::text_field;
use crate::spend::visibility::roles::is_admin_view_safe;
use crate::team_access::caller::TeamCaller;

use super::GroupModels;
use super::error::ScopeError;
use super::reads::TeamScopeReads;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamModelScope {
    pub models: Vec<String>,
}

#[must_use]
pub fn team_models_resolve_to_names(
    team_models: &[String],
    access_groups: &GroupModels,
) -> Vec<String> {
    let mut resolved = Vec::new();
    for name in team_models {
        match access_groups.get(name) {
            Some(members) => resolved.extend(members.iter().cloned()),
            None => resolved.push(name.clone()),
        }
    }
    resolved
}

pub async fn load_team_for_model_filter<Reads>(
    team_id: &str,
    reads: &Reads,
) -> Option<TeamModelScope>
where
    Reads: TeamScopeReads + ?Sized,
{
    match reads.team_models(team_id).await {
        Ok(Some(models)) => Some(TeamModelScope { models }),
        Ok(None) => {
            tracing::warn!("Team {team_id} not found in database");
            None
        }
        Err(error) => {
            tracing::error!("Error fetching team {team_id}: {error}");
            None
        }
    }
}

pub async fn gather_accessible_model_ids<Reads>(
    team_object: &TeamModelScope,
    team_id: &str,
    reads: &Reads,
    router: Option<&dyn HealthRouter>,
) -> Result<BTreeSet<String>, ScopeError>
where
    Reads: TeamScopeReads + ?Sized,
{
    let access_groups = match router {
        Some(health) => health.get_model_access_groups()?,
        None => GroupModels::new(),
    };
    let mut accessible = router_accessible_model_ids(team_object, team_id, &access_groups, router)?;
    let all_proxy_models = team_object
        .models
        .iter()
        .any(|name| name == ALL_PROXY_MODELS);
    if !team_object.models.is_empty() && !all_proxy_models {
        let resolved = team_models_resolve_to_names(&team_object.models, &access_groups);
        match reads.model_ids_by_names(&resolved).await {
            Ok(ids) => accessible.extend(ids.into_iter().filter(|id| !id.is_empty())),
            Err(error) => {
                tracing::debug!("Error querying database models for team {team_id}: {error}");
            }
        }
    }
    Ok(accessible)
}

fn router_accessible_model_ids(
    team_object: &TeamModelScope,
    team_id: &str,
    access_groups: &GroupModels,
    router: Option<&dyn HealthRouter>,
) -> Result<BTreeSet<String>, ScopeError> {
    let Some(health) = router else {
        return Ok(BTreeSet::new());
    };
    let all_proxy_models = team_object
        .models
        .iter()
        .any(|name| name == ALL_PROXY_MODELS);
    if team_object.models.is_empty() || all_proxy_models {
        return unrestricted_model_ids(team_id, health);
    }
    let resolved: BTreeSet<String> =
        team_models_resolve_to_names(&team_object.models, access_groups)
            .into_iter()
            .collect();
    let mut accessible = BTreeSet::new();
    for model_name in &resolved {
        for model in health.get_model_list(Some(model_name.as_str()), Some(team_id))? {
            if let Some(model_id) = text_field(&model.model_info, "id") {
                accessible.insert(model_id.to_owned());
            }
        }
    }
    Ok(accessible)
}

fn unrestricted_model_ids(
    team_id: &str,
    router: &dyn HealthRouter,
) -> Result<BTreeSet<String>, ScopeError> {
    let mut accessible = BTreeSet::new();
    for model in router.get_model_list(None, None)? {
        let Some(model_id) = text_field(&model.model_info, "id") else {
            continue;
        };
        let owner = text_field(&model.model_info, "team_id");
        if owner.is_none() || owner == Some(team_id) {
            accessible.insert(model_id.to_owned());
        }
    }
    Ok(accessible)
}

pub async fn authorize_team_id_query<Reads>(
    team_id: &str,
    caller: &TeamCaller,
    reads: &Reads,
) -> Result<(), ScopeError>
where
    Reads: TeamScopeReads + ?Sized,
{
    if is_admin_view_safe(caller) {
        return Ok(());
    }
    let Some(user_id) = caller.user_id.as_deref() else {
        return Err(ScopeError::NotAuthorized);
    };
    let stored = match reads.user_teams(user_id).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::error!(
                "Failed to look up caller teams while authorizing teamId filter: {error}"
            );
            return Err(ScopeError::NotAuthorized);
        }
    };
    let member = stored.is_some_and(|teams| teams.iter().any(|team| team == team_id));
    if member {
        return Ok(());
    }
    Err(ScopeError::NotAuthorized)
}
