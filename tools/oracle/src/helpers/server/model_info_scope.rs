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
    match input.function {
        Predicate::GrantResolution => {
            let snapshot = fixture(input)?;
            let groups = AccessGroups::default();
            let ids = grant_deployment_ids(&input.grant, &snapshot.lookup(), &groups)?;
            Ok(json!({"ids": ids}))
        }
        Predicate::DirectAccess => {
            let user = input.user.as_ref().ok_or(Failure::MissingUser)?;
            let snapshot = fixture(input)?;
            let groups = AccessGroups::default();
            let ids =
                direct_access_models(&user.models, &input.key_models, &snapshot.lookup(), &groups)?;
            Ok(json!({"ids": ids}))
        }
        Predicate::TeamModels => {
            let snapshot = fixture(input)?;
            let store = reads(input);
            let groups = AccessGroups::default();
            let selection = match &input.user_teams {
                Some(UserTeams::Ids(ids)) => TeamSelection::Ids(ids.clone()),
                Some(UserTeams::Star(marker)) if marker.as_str() == ALL_TEAMS => TeamSelection::All,
                Some(UserTeams::Star(marker)) => {
                    return Err(Failure::UnknownTeams(marker.clone()));
                }
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
        Predicate::NonAdmin => {
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
        Predicate::Populate => {
            let snapshot = fixture(input)?;
            let store = reads(input);
            let groups = AccessGroups::default();
            let models = runtime.block_on(accessible_models(
                specs(&input.all_models),
                false,
                &caller(input)?,
                &store,
                &snapshot.lookup(),
                &groups,
            ))?;
            Ok(spec_ids(&models))
        }
        Predicate::TeamAndDirect => {
            let snapshot = fixture(input)?;
            let store = reads(input);
            let groups = AccessGroups::default();
            let models = runtime.block_on(accessible_models(
                specs(&input.all_models),
                true,
                &caller(input)?,
                &store,
                &snapshot.lookup(),
                &groups,
            ))?;
            Ok(spec_ids(&models))
        }
        Predicate::Filter => {
            let kept = visible_models(specs(&input.all_models));
            Ok(spec_ids(&kept))
        }
    }
}
