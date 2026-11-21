use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::management_admins::invite::org_admin_can_invite_user;
use betterlite_server::management_admins::scope::{AdminCaller, is_team_admin};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Case {
    OrgAdminCanInviteUser(OrgCall),
    IsTeamAdmin(RosterCall),
}

#[derive(Debug, Deserialize)]
struct OrgCall {
    admin: JsonData,
    target: JsonData,
}

#[derive(Debug, Deserialize)]
struct RosterCall {
    #[serde(default)]
    caller_user_id: Option<String>,
    team: JsonData,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut outcomes: Vec<bool> = Vec::with_capacity(input.cases.len());
        for case in &input.cases {
            outcomes.push(outcome(case)?);
        }
        serde_json::to_value(outcomes).or_value_error()
    }
}

fn outcome(case: &Case) -> Result<bool, Box<HelperError>> {
    match case {
        Case::OrgAdminCanInviteUser(call) => {
            let admin = user_model(&call.admin)?;
            let target = user_model(&call.target)?;
            Ok(org_admin_can_invite_user(&admin, &target))
        }
        Case::IsTeamAdmin(call) => {
            let caller = AdminCaller {
                user_id: call.caller_user_id.as_deref(),
                proxy_admin: false,
            };
            let team = LiteLLMTeamTable::try_from(call.team.clone()).or_value_error()?;
            Ok(is_team_admin(&caller, &team))
        }
    }
}

fn user_model(record: &JsonData) -> Result<LiteLLMUserTable, Box<HelperError>> {
    match user_record_to_model(Some(record)).or_value_error()? {
        Some(user) => Ok(user),
        None => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: "a case user must be a row object".to_owned(),
        })),
    }
}
