use std::future::Future;
use std::pin::Pin;

use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use serde::Deserialize;

use super::annotate::{filter_models_to_user_accessible, populate_team_access};
use super::error::ScopeError;
use super::grants::{direct_access_models, non_team_model_ids, text_models};
use super::teams::{
    TeamModelLists, TeamScope, access_group_ids_for_teams, add_access_group_models,
    add_team_models_to_all_models, check_if_model_is_team_model,
};
use crate::model_reload::dedupe::deduplicate_router_models;

pub type ScopeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCreator {
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserScope {
    pub teams: Vec<String>,
    pub models: Vec<JsonData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamSelection {
    All,
    Ids(Vec<String>),
}

pub trait ScopeReads: Send + Sync {
    fn model_creator<'a>(
        &'a self,
        model_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<ModelCreator>, ScopeError>>;

    fn user_scope<'a>(
        &'a self,
        user_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<UserScope>, ScopeError>>;

    fn team_scopes<'a>(
        &'a self,
        selection: &'a TeamSelection,
    ) -> ScopeFuture<'a, Result<Vec<TeamScope>, ScopeError>>;

    fn access_group_models<'a>(
        &'a self,
        access_group_ids: &'a [String],
    ) -> ScopeFuture<'a, Result<AccessGroupMap, ScopeError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeCaller {
    pub user_id: Option<String>,
    pub key_models: Vec<String>,
    pub admin_view: bool,
}

#[derive(Deserialize)]
struct CallerFields {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    models: Vec<JsonData>,
}

impl ScopeCaller {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, ScopeError> {
        let fields: CallerFields = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(ScopeError::Caller)?;
        Ok(Self {
            user_id: fields.user_id,
            key_models: text_models(&fields.models, "key models")?,
            admin_view: user_api_key_has_admin_view(auth),
        })
    }
}

pub async fn check_if_model_is_user_added(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    reads: Option<&dyn ScopeReads>,
) -> Result<Vec<DeploymentSpec>, ScopeError> {
    let Some(reads) = reads else {
        return Err(ScopeError::DatabaseUnavailable);
    };
    let mut kept = Vec::new();
    for model in all_models {
        let Some(model_id) = model.id().map(str::to_owned) else {
            continue;
        };
        let saved = reads.model_creator(&model_id).await?;
        if saved.is_some_and(|creator| creator.created_by.as_deref() == caller.user_id.as_deref()) {
            kept.push(model);
        }
    }
    Ok(kept)
}

pub async fn non_admin_all_models(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    reads: Option<&dyn ScopeReads>,
    lookup: &CatalogLookup<'_>,
) -> Result<Vec<DeploymentSpec>, ScopeError> {
    let Some(reads) = reads else {
        return Err(ScopeError::DatabaseUnavailable);
    };
    let mut models = check_if_model_is_user_added(all_models, caller, Some(reads)).await?;
    if let Some(user_id) = caller.user_id.as_deref().filter(|value| !value.is_empty()) {
        let user = reads
            .user_scope(user_id)
            .await
            .map_err(|_read_error| ScopeError::UserNotFound)?;
        if let Some(user) = user {
            let deployed = lookup.get_model_list(None, None)?;
            models.extend(check_if_model_is_team_model(&deployed, &user.teams));
        }
    }
    Ok(deduplicate_router_models(&models))
}
pub async fn get_all_team_models(
    selection: &TeamSelection,
    reads: &dyn ScopeReads,
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<TeamModelLists, ScopeError> {
    let teams = reads.team_scopes(selection).await?;
    let mut team_models = add_team_models_to_all_models(&teams, lookup, access_groups)?;
    let group_ids = access_group_ids_for_teams(&teams);
    if !group_ids.is_empty() {
        let group_models = reads.access_group_models(&group_ids).await?;
        add_access_group_models(&teams, lookup, &group_models, &mut team_models)?;
    }
    let returned: TeamModelLists = team_models
        .into_iter()
        .map(|(model_id, team_ids)| (model_id, team_ids.into_iter().collect()))
        .collect();
    Ok(returned)
}
async fn caller_access(
    caller: &ScopeCaller,
    reads: &dyn ScopeReads,
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<(Option<TeamModelLists>, Vec<String>), ScopeError> {
    if caller.admin_view {
        let teams = get_all_team_models(&TeamSelection::All, reads, lookup, access_groups).await?;
        return Ok((Some(teams), non_team_model_ids(lookup)));
    }
    let Some(user_id) = caller.user_id.as_deref() else {
        return Ok((None, Vec::new()));
    };
    let Some(user) = reads.user_scope(user_id).await? else {
        return Ok((None, Vec::new()));
    };
    let user_models = text_models(&user.models, "user models")?;
    let direct = direct_access_models(&user_models, &caller.key_models, lookup, access_groups)?;
    let teams = get_all_team_models(
        &TeamSelection::Ids(user.teams),
        reads,
        lookup,
        access_groups,
    )
    .await?;
    Ok((Some(teams), direct))
}
pub async fn populate_team_access_on_models(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    reads: &dyn ScopeReads,
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<Vec<DeploymentSpec>, ScopeError> {
    let (team_models, direct_ids) = caller_access(caller, reads, lookup, access_groups).await?;
    let mut annotated = all_models;
    populate_team_access(&mut annotated, team_models.as_ref(), &direct_ids);
    Ok(annotated)
}

pub async fn get_all_team_and_direct_access_models(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    reads: &dyn ScopeReads,
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<Vec<DeploymentSpec>, ScopeError> {
    let annotated =
        populate_team_access_on_models(all_models, caller, reads, lookup, access_groups).await?;
    Ok(filter_models_to_user_accessible(annotated))
}
