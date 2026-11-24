use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use serde::Deserialize;

use super::annotate::{populate_team_access, visible_models};
use super::error::ScopeError;
use super::grants::{direct_access_models, non_team_model_ids, text_models};
use super::teams::{
    TeamModelLists, TeamScope, access_group_ids_for_teams, add_access_group_models,
    add_team_models_to_all_models, check_if_model_is_team_model,
};
use crate::model_reload::dedupe::deduplicate_router_models;

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

// The reads each policy makes before its catalog lookups run; the lookup itself borrows a catalog that is not Sync, so no future may hold it across an await.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserModels {
    pub models: Vec<DeploymentSpec>,
    pub user: Option<UserScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamReads {
    pub teams: Vec<TeamScope>,
    pub group_models: AccessGroupMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerRows {
    Admin(TeamReads),
    User {
        models: Vec<JsonData>,
        teams: TeamReads,
    },
    Unresolved,
}

#[async_trait]
pub trait ScopeReads: Send + Sync {
    async fn model_creator(&self, model_id: &str) -> Result<Option<ModelCreator>, ScopeError>;
    async fn user_scope(&self, user_id: &str) -> Result<Option<UserScope>, ScopeError>;
    async fn team_scopes(&self, selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError>;
    async fn access_group_models(
        &self,
        access_group_ids: &[String],
    ) -> Result<AccessGroupMap, ScopeError>;
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

    // _populate_team_access_on_models, then the filter get_all_team_and_direct_access_models applies: annotate from the caller's rows through the lookups tuple of the catalog and its access-group cache, and keep what it can use when visible_only is set.
    pub fn accessible_models(
        &self,
        annotated: Vec<DeploymentSpec>,
        visible_only: bool,
        rows: CallerRows,
        lookups: (&CatalogLookup<'_>, &AccessGroups),
    ) -> Result<Vec<DeploymentSpec>, ScopeError> {
        let (lookup, access_groups) = lookups;
        let mut models = annotated;
        match rows {
            CallerRows::Admin(teams) => {
                let direct = non_team_model_ids(lookup);
                let team_models = get_all_team_models(teams, lookup, access_groups)?;
                populate_team_access(&mut models, Some(&team_models), &direct);
            }
            CallerRows::User {
                models: granted,
                teams,
            } => {
                let user_models = text_models(&granted, "user models")?;
                let direct =
                    direct_access_models(&user_models, &self.key_models, lookup, access_groups)?;
                let team_models = get_all_team_models(teams, lookup, access_groups)?;
                populate_team_access(&mut models, Some(&team_models), &direct);
            }
            CallerRows::Unresolved => populate_team_access(&mut models, None, &[]),
        }
        if visible_only {
            return Ok(visible_models(models));
        }
        Ok(models)
    }
}

// The rows whose stored created_by names the caller; a row without an id can never match.
pub async fn check_if_model_is_user_added<Store>(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    store: &Store,
) -> Result<Vec<DeploymentSpec>, ScopeError>
where
    Store: ScopeReads + ?Sized,
{
    let mut kept = Vec::new();
    for model in all_models {
        let Some(model_id) = model.id().map(str::to_owned) else {
            continue;
        };
        let saved = store.model_creator(&model_id).await?;
        if saved.is_some_and(|creator| creator.created_by.as_deref() == caller.user_id.as_deref()) {
            kept.push(model);
        }
    }
    Ok(kept)
}

// The reads non_admin_all_models makes: the caller's own rows, then the stored user row a truthy user_id names, whose failed read is LiteLLM's 400.
pub async fn read_user_models<Store>(
    all_models: Vec<DeploymentSpec>,
    caller: &ScopeCaller,
    reads: Option<&Store>,
) -> Result<UserModels, ScopeError>
where
    Store: ScopeReads + ?Sized,
{
    let store = reads.ok_or(ScopeError::DatabaseUnavailable)?;
    let models = check_if_model_is_user_added(all_models, caller, store).await?;
    let Some(user_id) = caller.user_id.as_deref().filter(|value| !value.is_empty()) else {
        return Ok(UserModels { models, user: None });
    };
    let user = store
        .user_scope(user_id)
        .await
        .map_err(|read_error| ScopeError::UserNotFound(Box::new(read_error)))?;
    Ok(UserModels { models, user })
}

// non_admin_all_models: the caller's own rows, then every router row whose team_id the stored user's teams name.
pub fn non_admin_all_models(
    stored: UserModels,
    lookup: &CatalogLookup<'_>,
) -> Result<Vec<DeploymentSpec>, ScopeError> {
    let UserModels { mut models, user } = stored;
    if let Some(stored_user) = user {
        let deployed = lookup.get_model_list(None, None)?;
        models.extend(check_if_model_is_team_model(&deployed, &stored_user.teams));
    }
    Ok(deduplicate_router_models(&models))
}

// The batch reads get_all_team_models makes: the teams the selection names, then the access groups their restricted members read.
pub async fn read_team_models<Store>(
    selection: &TeamSelection,
    reads: &Store,
) -> Result<TeamReads, ScopeError>
where
    Store: ScopeReads + ?Sized,
{
    let teams = reads.team_scopes(selection).await?;
    let group_ids = access_group_ids_for_teams(&teams);
    let group_models = if group_ids.is_empty() {
        AccessGroupMap::new()
    } else {
        reads.access_group_models(&group_ids).await?
    };
    Ok(TeamReads {
        teams,
        group_models,
    })
}

// get_all_team_models: each team's own models, then the models its access groups name.
pub fn get_all_team_models(
    read: TeamReads,
    lookup: &CatalogLookup<'_>,
    access_groups: &AccessGroups,
) -> Result<TeamModelLists, ScopeError> {
    let TeamReads {
        teams,
        group_models,
    } = read;
    let mut team_models = add_team_models_to_all_models(&teams, lookup, access_groups)?;
    add_access_group_models(lookup, &teams, &group_models, &mut team_models)?;
    let returned: TeamModelLists = team_models
        .into_iter()
        .map(|(model_id, team_ids)| (model_id, team_ids.into_iter().collect()))
        .collect();
    Ok(returned)
}

// The reads _populate_team_access_on_models makes: an admin view names every team, a caller reads its stored row and that row's teams, and a caller without a stored row reads nothing.
pub async fn read_caller_rows<Store>(
    caller: &ScopeCaller,
    reads: &Store,
) -> Result<CallerRows, ScopeError>
where
    Store: ScopeReads + ?Sized,
{
    if caller.admin_view {
        let teams = read_team_models(&TeamSelection::All, reads).await?;
        return Ok(CallerRows::Admin(teams));
    }
    let Some(user_id) = caller.user_id.as_deref() else {
        return Ok(CallerRows::Unresolved);
    };
    let Some(user) = reads.user_scope(user_id).await? else {
        return Ok(CallerRows::Unresolved);
    };
    let teams = read_team_models(&TeamSelection::Ids(user.teams), reads).await?;
    Ok(CallerRows::User {
        models: user.models,
        teams,
    })
}

// clippy::pub_use is denied and no expect may be added, and the oracle helper crate may not depend on betterlite-router, so these aliases keep the scope's deployment and group types nameable there
pub type Deployment = DeploymentSpec;
pub type GroupLookups = AccessGroups;
pub type GroupModels = AccessGroupMap;
