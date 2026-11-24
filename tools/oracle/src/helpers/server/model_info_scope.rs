mod fixture;
mod reads;
mod records;

use betterlite_server::model_info_scope::annotate::visible_models;
use betterlite_server::model_info_scope::compose::GroupLookups;
use betterlite_server::model_info_scope::compose::{
    ScopeCaller, TeamSelection, get_all_team_models, non_admin_all_models, read_caller_rows,
    read_team_models, read_user_models,
};
use betterlite_server::model_info_scope::grants::{direct_access_models, grant_deployment_ids};
use betterlite_server::model_visibility::testing::Snapshot;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) use records::{CallerRecord, Failure, Input, Predicate, UserTeams};

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

impl From<&CallerRecord> for ScopeCaller {
    fn from(record: &CallerRecord) -> Self {
        Self {
            user_id: record.user_id.clone(),
            key_models: record.models.clone(),
            admin_view: record.admin_view,
        }
    }
}

fn selection(input: &Input) -> Result<TeamSelection, Failure> {
    match &input.user_teams {
        Some(UserTeams::Ids(ids)) => Ok(TeamSelection::Ids(ids.clone())),
        Some(UserTeams::Star(marker)) if marker.as_str() == ALL_TEAMS => Ok(TeamSelection::All),
        Some(UserTeams::Star(marker)) => Err(Failure::UnknownTeams(marker.clone())),
        None => Ok(TeamSelection::All),
    }
}

// The reads each predicate drives through the current-thread runtime, and the catalog lookups they feed.
fn answer(input: &Input) -> Result<Value, Failure> {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let snapshot = fixture(input)?;
    match input.function {
        Predicate::GrantResolution => {
            let ids =
                grant_deployment_ids(&input.grant, &snapshot.lookup(), &GroupLookups::default())?;
            Ok(json!({"ids": ids}))
        }
        Predicate::DirectAccess => direct_ids(input, &snapshot),
        Predicate::TeamModels => {
            let read = runtime.block_on(read_team_models(&selection(input)?, &reads(input)))?;
            Ok(team_map(&get_all_team_models(
                read,
                &snapshot.lookup(),
                &GroupLookups::default(),
            )?))
        }
        Predicate::NonAdmin => {
            let record = input.caller.as_ref().ok_or(Failure::MissingCaller)?;
            let alice = ScopeCaller::from(record);
            let read = runtime.block_on(read_user_models(
                specs(&input.all_models),
                &alice,
                Some(&reads(input)),
            ))?;
            Ok(spec_ids(&non_admin_all_models(read, &snapshot.lookup())?))
        }
        Predicate::Populate => populate(input, &snapshot, &runtime, false),
        Predicate::TeamAndDirect => populate(input, &snapshot, &runtime, true),
        Predicate::Filter => Ok(spec_ids(&visible_models(specs(&input.all_models)))),
    }
}

fn direct_ids(input: &Input, snapshot: &Snapshot) -> Result<Value, Failure> {
    let user = input.user.as_ref().ok_or(Failure::MissingUser)?;
    let lookup = snapshot.lookup();
    let groups = GroupLookups::default();
    let ids = direct_access_models(&user.models, &input.key_models, &lookup, &groups)?;
    Ok(json!({"ids": ids}))
}

fn populate(
    input: &Input,
    snapshot: &Snapshot,
    runtime: &Runtime,
    visible_only: bool,
) -> Result<Value, Failure> {
    let record = input.caller.as_ref().ok_or(Failure::MissingCaller)?;
    let alice = ScopeCaller::from(record);
    let read = runtime.block_on(read_caller_rows(&alice, &reads(input)))?;
    let models = alice.accessible_models(
        specs(&input.all_models),
        visible_only,
        read,
        (&snapshot.lookup(), &GroupLookups::default()),
    )?;
    Ok(spec_ids(&models))
}
