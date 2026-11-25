mod reads;
mod records;

use std::collections::BTreeSet;

use betterlite_auth::user_roles::UserRole;
use betterlite_server::model_team_scope::caller_scope::{
    byok_row_outside_caller_teams, get_caller_byok_team_scope,
};
use betterlite_server::model_team_scope::team_models::team_models_resolve_to_names;
use betterlite_server::team_access::caller::TeamCaller;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) use records::{CallerRecord, Failure, Function, Input};

use reads::reads;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

impl From<&CallerRecord> for TeamCaller {
    fn from(record: &CallerRecord) -> Self {
        Self {
            user_id: record.user_id.clone(),
            user_role: record.admin_view.then_some(UserRole::ProxyAdmin),
            team_id: record.team_id.clone(),
            team_service_account: false,
        }
    }
}

fn caller_scope(input: &Input, runtime: &Runtime) -> Value {
    let caller = input.caller.as_ref().map(TeamCaller::from);
    let reads = reads(input);
    let scope = runtime.block_on(get_caller_byok_team_scope(caller.as_ref(), Some(&reads)));
    json!({"allowed_team_ids": scope})
}

fn answer(input: &Input) -> Result<Value, Failure> {
    match input.function {
        Function::ByokRowOutsideCallerTeams => {
            let allowed: Option<BTreeSet<String>> = input
                .allowed_team_ids
                .as_ref()
                .map(|ids| ids.iter().cloned().collect());
            Ok(json!({
                "outside": byok_row_outside_caller_teams(&input.model_info, allowed.as_ref()),
            }))
        }
        Function::TeamModelsResolveToNames => {
            let resolved = team_models_resolve_to_names(&input.team_models, &input.access_groups);
            Ok(json!({"names": resolved}))
        }
        Function::CallerByokTeamScope => {
            let runtime = Builder::new_current_thread().enable_all().build()?;
            Ok(caller_scope(input, &runtime))
        }
    }
}
