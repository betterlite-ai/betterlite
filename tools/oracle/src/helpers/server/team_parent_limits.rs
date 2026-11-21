use std::fmt::Display;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_parent_limits::allocation::{
    check_org_team_model_specific_limits, check_org_team_rpm_tpm_limits,
};
use betterlite_server::team_parent_limits::error::{
    REJECTION_STATUS, RejectionDetail, TeamLimitError,
};
use betterlite_server::team_parent_limits::request::TeamLimitRequest;
use betterlite_server::team_request_contracts::create::NewTeamRequest;
use betterlite_server::team_request_contracts::update::UpdateTeamRequest;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Check {
    ModelSpecific,
    RpmTpm,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
enum TeamRequest {
    New(JsonData),
    Update(JsonData),
}

#[derive(Deserialize)]
struct Call {
    check: Check,
    teams: Vec<LiteLLMTeamTable>,
    organization: OrganizationTable,
    request: TeamRequest,
}

#[derive(Serialize)]
struct Outcome {
    status: Option<u16>,
    detail: Option<RejectionDetail>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut outcomes = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let outcome = match &call.request {
                TeamRequest::New(body) => {
                    let parsed = NewTeamRequest::from_value(body).map_err(failure)?;
                    judge(call, &TeamLimitRequest::from(&parsed))?
                }
                TeamRequest::Update(body) => {
                    let parsed = UpdateTeamRequest::from_value(body).map_err(failure)?;
                    judge(call, &TeamLimitRequest::from(&parsed))?
                }
            };
            outcomes.push(outcome);
        }
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn judge(call: &Call, request: &TeamLimitRequest<'_>) -> Result<Outcome, Box<HelperError>> {
    let (teams, organization) = (&call.teams, &call.organization);
    let result = match call.check {
        Check::ModelSpecific => check_org_team_model_specific_limits(teams, organization, request),
        Check::RpmTpm => check_org_team_rpm_tpm_limits(teams, organization, request)
            .map_err(TeamLimitError::from),
    };
    match result {
        Ok(()) => Ok(Outcome {
            status: None,
            detail: None,
        }),
        Err(TeamLimitError::Rejected(rejection)) => Ok(Outcome {
            status: Some(REJECTION_STATUS.as_u16()),
            detail: Some(rejection.detail()),
        }),
        Err(
            error @ (TeamLimitError::ModelLimits { .. }
            | TeamLimitError::Caller(_)
            | TeamLimitError::MissingOrganization
            | TeamLimitError::Database(_)
            | TeamLimitError::TeamQuery(_)
            | TeamLimitError::UserLookup(_)),
        ) => Err(failure(error)),
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "TeamParentLimits".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
