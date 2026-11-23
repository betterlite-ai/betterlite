use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_auth::agent_caller::caller_auth::caller_record;
use betterlite_auth::agent_policy::caller::AgentCaller;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::{is_ui_session_credential, resolve_ui_session_team_ids};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use ports::CaseTeams;

mod ports;
mod tests;

// the case names its helper operation so a case copied under another helper fails to parse
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Session {
        auth: AuthFields,
    },
    TeamIds {
        auth: AuthFields,
        database: bool,
        lookup: LookupAnswer,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthFields {
    team_id: Option<String>,
    user_id: Option<String>,
    requires_fresh_policy: bool,
}

// what the scripted user read answers
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LookupAnswer {
    Teams { teams: Vec<String> },
    NoUser,
    Failure,
}

impl AuthFields {
    // LiteLLM sets requires_fresh_policy past the validator that strips it from constructor input
    fn token(&self) -> Result<UserAPIKeyAuth, Box<HelperError>> {
        let caller = AgentCaller {
            user_id: self.user_id.clone(),
            team_id: self.team_id.clone(),
        };
        caller_record(&caller, self.requires_fresh_policy, None).or_value_error()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Session { auth } => session_case(auth),
            Input::TeamIds {
                auth,
                database,
                lookup,
            } => team_ids_case(auth, *database, lookup),
        }
    }
}

#[derive(Serialize)]
struct SessionAnswer {
    session: bool,
}

fn session_case(fields: &AuthFields) -> HelperResult {
    let auth = fields.token()?;
    let answer = SessionAnswer {
        session: is_ui_session_credential(&auth),
    };
    serde_json::to_value(answer).or_value_error()
}

#[derive(Serialize)]
struct TeamIdsAnswer {
    teams: Vec<String>,
    lookup_ran: bool,
    fresh: Option<bool>,
}

fn team_ids_case(fields: &AuthFields, database: bool, lookup: &LookupAnswer) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let auth = fields.token()?;
    let teams = CaseTeams {
        database,
        lookup,
        ran: AtomicBool::new(false),
        fresh: AtomicBool::new(false),
    };
    let resolved = runtime.block_on(resolve_ui_session_team_ids(&auth, &teams));
    let lookup_ran = teams.ran.load(Ordering::SeqCst);
    let fresh = lookup_ran.then_some(teams.fresh.load(Ordering::SeqCst));
    let answer = TeamIdsAnswer {
        teams: resolved,
        lookup_ran,
        fresh,
    };
    serde_json::to_value(answer).or_value_error()
}
