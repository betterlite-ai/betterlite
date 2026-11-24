mod fixture;
mod reads;
mod records;

use betterlite_router::model_group_lookups::access_groups::AccessGroups;
use betterlite_server::model_info_scope::annotate::visible_models;
use betterlite_server::model_info_scope::compose::{
    ScopeCaller, TeamSelection, accessible_models, get_all_team_models, non_admin_all_models,
};
use betterlite_server::model_info_scope::grants::{direct_access_models, grant_deployment_ids};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) use records::{Failure, Input, Predicate, UserTeams};

use fixture::{fixture, spec_ids, specs, team_map};
use reads::reads;

const ALL_TEAMS: &str = "*";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
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

fn answer(input: &Input) -> Result<Value, Failure> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(resolve(input))
}

async fn resolve(input: &Input) -> Result<Value, Failure> {
    match input.function {
        Predicate::GrantResolution => grant_answer(input),
        Predicate::DirectAccess => direct_answer(input),
        Predicate::TeamModels => {
            let snapshot = fixture(input)?;
            let found = get_all_team_models(
                &selection(input)?,
                &reads(input),
                &snapshot.lookup(),
                &AccessGroups::default(),
            )
            .await?;
            Ok(team_map(&found))
        }
        Predicate::NonAdmin => {
            let snapshot = fixture(input)?;
            let models = non_admin_all_models(
                specs(&input.all_models),
                &caller(input)?,
                Some(&reads(input)),
                &snapshot.lookup(),
            )
            .await?;
            Ok(spec_ids(&models))
        }
        Predicate::Populate => populate(input, false).await,
        Predicate::TeamAndDirect => populate(input, true).await,
        Predicate::Filter => Ok(spec_ids(&visible_models(specs(&input.all_models)))),
    }
}

fn grant_answer(input: &Input) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let ids = grant_deployment_ids(&input.grant, &snapshot.lookup(), &AccessGroups::default())?;
    Ok(json!({"ids": ids}))
}

fn direct_answer(input: &Input) -> Result<Value, Failure> {
    let user = input.user.as_ref().ok_or(Failure::MissingUser)?;
    let snapshot = fixture(input)?;
    let ids = direct_access_models(
        &user.models,
        &input.key_models,
        &snapshot.lookup(),
        &AccessGroups::default(),
    )?;
    Ok(json!({"ids": ids}))
}

fn selection(input: &Input) -> Result<TeamSelection, Failure> {
    match &input.user_teams {
        Some(UserTeams::Ids(ids)) => Ok(TeamSelection::Ids(ids.clone())),
        Some(UserTeams::Star(marker)) if marker.as_str() == ALL_TEAMS => Ok(TeamSelection::All),
        Some(UserTeams::Star(marker)) => Err(Failure::UnknownTeams(marker.clone())),
        None => Ok(TeamSelection::All),
    }
}

async fn populate(input: &Input, visible_only: bool) -> Result<Value, Failure> {
    let snapshot = fixture(input)?;
    let models = accessible_models(
        specs(&input.all_models),
        visible_only,
        &caller(input)?,
        &reads(input),
        &snapshot.lookup(),
        &AccessGroups::default(),
    )
    .await?;
    Ok(spec_ids(&models))
}
