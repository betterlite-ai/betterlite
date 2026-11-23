use std::collections::BTreeMap;
use std::io;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use betterlite_server::model_info_scope::annotate::filter_models_to_user_accessible;
use betterlite_server::model_info_scope::compose::{
    ModelCreator, ScopeCaller, ScopeFuture, ScopeReads, TeamSelection, UserScope,
    get_all_team_and_direct_access_models, get_all_team_models, non_admin_all_models,
    populate_team_access_on_models,
};
use betterlite_server::model_info_scope::error::ScopeError;
use betterlite_server::model_info_scope::grants::{
    direct_access_models, resolve_model_grant_to_deployment_ids,
};
use betterlite_server::model_info_scope::teams::{TeamModelLists, TeamScope};
use betterlite_server::model_visibility::testing::{FixtureError, Row as SnapshotRow, Snapshot};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const ALL_TEAMS: &str = "*";

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Fixture(#[from] FixtureError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error("the case carries no caller record")]
    MissingCaller,
    #[error("the direct_access function needs a user row")]
    MissingUser,
    #[error("user_teams is \"*\" or a list of team ids, not {0}")]
    UnknownTeams(String),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Predicate {
    NonAdmin,
    TeamModels,
    GrantResolution,
    DirectAccess,
    Populate,
    TeamAndDirect,
    Filter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    model_name: String,
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserRecord {
    user_id: String,
    #[serde(default)]
    teams: Vec<String>,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamRecord {
    team_id: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    access_group_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessGroupRecord {
    access_group_id: String,
    #[serde(default)]
    access_model_names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerRecord {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    admin_view: bool,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UserTeams {
    Star(String),
    Ids(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Predicate,
    #[serde(default)]
    model_list: Vec<RawRow>,
    #[serde(default)]
    model_group_alias: BTreeMap<String, String>,
    #[serde(default)]
    all_models: Vec<RawRow>,
    #[serde(default)]
    grant: Vec<String>,
    #[serde(default)]
    key_models: Vec<String>,
    #[serde(default)]
    user: Option<UserRecord>,
    #[serde(default)]
    creators: BTreeMap<String, Option<String>>,
    #[serde(default)]
    teams: Vec<TeamRecord>,
    #[serde(default)]
    user_teams: Option<UserTeams>,
    #[serde(default)]
    access_groups: Vec<AccessGroupRecord>,
    #[serde(default)]
    caller: Option<CallerRecord>,
}

#[derive(Default)]
struct CaseReads {
    creators: BTreeMap<String, Option<String>>,
    user_id: Option<String>,
    user: Option<UserScope>,
    teams: Vec<TeamScope>,
    groups: AccessGroupMap,
}

impl ScopeReads for CaseReads {
    fn model_creator<'a>(
        &'a self,
        model_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<ModelCreator>, ScopeError>> {
        Box::pin(async move {
            let creator = self.creators.get(model_id).map(|created_by| ModelCreator {
                created_by: created_by.clone(),
            });
            Ok(creator)
        })
    }

    fn user_scope<'a>(
        &'a self,
        user_id: &'a str,
    ) -> ScopeFuture<'a, Result<Option<UserScope>, ScopeError>> {
        Box::pin(async move {
            let found = self
                .user
                .as_ref()
                .filter(|_stored| self.user_id.as_deref() == Some(user_id))
                .cloned();
            Ok(found)
        })
    }

    fn team_scopes<'a>(
        &'a self,
        _selection: &'a TeamSelection,
    ) -> ScopeFuture<'a, Result<Vec<TeamScope>, ScopeError>> {
        Box::pin(async move { Ok(self.teams.clone()) })
    }

    fn access_group_models<'a>(
        &'a self,
        _access_group_ids: &'a [String],
    ) -> ScopeFuture<'a, Result<AccessGroupMap, ScopeError>> {
        Box::pin(async move { Ok(self.groups.clone()) })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

fn fixture(input: &Input) -> Result<Snapshot, Failure> {
    let infos: Vec<Json> = input
        .model_list
        .iter()
        .map(|row| Json::Object(row.model_info.clone()))
        .collect();
    let rows: Vec<SnapshotRow<'_>> = input
        .model_list
        .iter()
        .zip(&infos)
        .map(|(row, model_info)| SnapshotRow {
            model_name: &row.model_name,
            litellm_params: &row.litellm_params,
            model_info,
        })
        .collect();
    let aliases: JsonObject = input
        .model_group_alias
        .iter()
        .map(|(alias, target)| (alias.clone(), Json::Text(target.clone())))
        .collect();
    Ok(Snapshot::build(&rows, &aliases)?)
}

fn specs(rows: &[RawRow]) -> Vec<DeploymentSpec> {
    rows.iter()
        .map(|row| DeploymentSpec {
            model_name: row.model_name.clone(),
            litellm_params: row.litellm_params.clone(),
            model_info: row.model_info.clone(),
            extra: JsonObject::new(),
        })
        .collect()
}

fn reads(input: &Input) -> CaseReads {
    let user = input.user.as_ref().map(|record| UserScope {
        teams: record.teams.clone(),
        models: record
            .models
            .iter()
            .map(|name| JsonData::String(name.clone()))
            .collect(),
    });
    CaseReads {
        creators: input.creators.clone(),
        user_id: input.user.as_ref().map(|record| record.user_id.clone()),
        user,
        teams: input
            .teams
            .iter()
            .map(|record| TeamScope {
                team_id: record.team_id.clone(),
                models: record.models.clone(),
                access_group_ids: record.access_group_ids.clone().unwrap_or_default(),
            })
            .collect(),
        groups: input
            .access_groups
            .iter()
            .map(|record| {
                (
                    record.access_group_id.clone(),
                    record.access_model_names.clone(),
                )
            })
            .collect(),
    }
}

fn caller(input: &Input) -> Result<ScopeCaller, Failure> {
    let record = input.caller.as_ref().ok_or(Failure::MissingCaller)?;
    Ok(ScopeCaller {
        user_id: record.user_id.clone(),
        key_models: record.models.clone(),
        admin_view: record.admin_view,
    })
}

fn spec_ids(models: &[DeploymentSpec]) -> Value {
    let ids: Vec<Value> = models
        .iter()
        .map(|model| {
            model
                .id()
                .map_or(Value::Null, |id| Value::String(id.to_owned()))
        })
        .collect();
    json!({"ids": ids})
}

fn team_map(found: &TeamModelLists) -> Value {
    let mut sorted: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (model_id, team_ids) in found {
        let mut names: Vec<&str> = team_ids.iter().map(String::as_str).collect();
        names.sort_unstable();
        sorted.insert(model_id.as_str(), names);
    }
    json!({"team_models": sorted})
}

fn grant_answer(input: &Input) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let groups = AccessGroups::default();
    let ids = resolve_model_grant_to_deployment_ids(&input.grant, &snapshot.lookup(), &groups)?;
    Ok(json!({"ids": ids}))
}

fn direct_answer(input: &Input) -> Result<Value, Failure> {
    let Some(user) = &input.user else {
        return Err(Failure::MissingUser);
    };
    let snapshot = fixture(input)?;
    let groups = AccessGroups::default();
    let ids = direct_access_models(&user.models, &input.key_models, &snapshot.lookup(), &groups)?;
    Ok(json!({"ids": ids}))
}

fn team_models_answer(input: &Input, runtime: &Runtime) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let store = reads(input);
    let groups = AccessGroups::default();
    let selection = match &input.user_teams {
        Some(UserTeams::Ids(ids)) => TeamSelection::Ids(ids.clone()),
        Some(UserTeams::Star(marker)) if marker.as_str() == ALL_TEAMS => TeamSelection::All,
        Some(UserTeams::Star(marker)) => return Err(Failure::UnknownTeams(marker.clone())),
        None => TeamSelection::All,
    };
    let found = runtime.block_on(get_all_team_models(
        &selection,
        &store,
        &snapshot.lookup(),
        &groups,
    ))?;
    Ok(team_map(&found))
}

fn non_admin_answer(input: &Input, runtime: &Runtime) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let store = reads(input);
    let models = runtime.block_on(non_admin_all_models(
        specs(&input.all_models),
        &caller(input)?,
        Some(&store),
        &snapshot.lookup(),
    ))?;
    Ok(spec_ids(&models))
}

fn populate_answer(input: &Input, runtime: &Runtime) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let store = reads(input);
    let groups = AccessGroups::default();
    let models = runtime.block_on(populate_team_access_on_models(
        specs(&input.all_models),
        &caller(input)?,
        &store,
        &snapshot.lookup(),
        &groups,
    ))?;
    Ok(spec_ids(&models))
}

fn team_and_direct_answer(input: &Input, runtime: &Runtime) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let store = reads(input);
    let groups = AccessGroups::default();
    let models = runtime.block_on(get_all_team_and_direct_access_models(
        specs(&input.all_models),
        &caller(input)?,
        &store,
        &snapshot.lookup(),
        &groups,
    ))?;
    Ok(spec_ids(&models))
}

fn answer(input: &Input) -> Result<Value, Failure> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    match input.function {
        Predicate::GrantResolution => grant_answer(input),
        Predicate::DirectAccess => direct_answer(input),
        Predicate::TeamModels => team_models_answer(input, &runtime),
        Predicate::NonAdmin => non_admin_answer(input, &runtime),
        Predicate::Populate => populate_answer(input, &runtime),
        Predicate::TeamAndDirect => team_and_direct_answer(input, &runtime),
        Predicate::Filter => {
            let kept = filter_models_to_user_accessible(specs(&input.all_models));
            Ok(spec_ids(&kept))
        }
    }
}
