mod tests;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::member_request_contracts::MemberContractError;
use betterlite_server::member_request_contracts::callbacks::{
    AddTeamCallback, TeamCallbackMetadata,
};
use betterlite_server::member_request_contracts::members::{
    self, MemberAddRequest, OrgMember, OrgMemberAddRequest, OrganizationMemberAddRequest,
    TeamMemberAddRequest, TeamMemberDeleteRequest, TeamMemberUpdateRequest,
};
use betterlite_server::member_request_contracts::organization::{
    OrganizationTableUpdate, OrganizationTableWithMembers, OrganizationUpdateRequestV2,
    PresentFields,
};
use betterlite_server::member_request_contracts::team::DeleteTeamRequest;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) enum Record {
    DeleteTeamRequest,
    AddTeamCallback,
    TeamCallbackMetadata,
    OrgMember,
    MemberAddRequest,
    TeamMemberAddRequest,
    OrgMemberAddRequest,
    OrganizationMemberAddRequest,
    TeamMemberDeleteRequest,
    // ast-grep-ignore: no-long-names
    OrganizationMemberDeleteRequest,
    TeamMemberUpdateRequest,
    // ast-grep-ignore: no-long-names
    OrganizationMemberUpdateRequest,
    OrganizationTableUpdate,
    OrganizationUpdateRequestV2,
    OrganizationTableWithMembers,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub record: Record,
    pub value: Json,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        outcome(&input.record, &input.value)
    }
}

fn outcome(record: &Record, value: &Json) -> HelperResult {
    match record {
        Record::DeleteTeamRequest => recorded(DeleteTeamRequest::from_value(value)),
        Record::AddTeamCallback => recorded(AddTeamCallback::from_value(value)),
        Record::TeamCallbackMetadata => recorded(TeamCallbackMetadata::from_value(value)),
        Record::OrgMember => recorded(OrgMember::from_value(value)),
        Record::MemberAddRequest => recorded(MemberAddRequest::from_value(value)),
        Record::TeamMemberAddRequest => recorded(TeamMemberAddRequest::from_value(value)),
        Record::OrgMemberAddRequest => recorded(OrgMemberAddRequest::from_value(value)),
        Record::OrganizationMemberAddRequest => {
            recorded(OrganizationMemberAddRequest::from_value(value))
        }
        Record::TeamMemberDeleteRequest => recorded(TeamMemberDeleteRequest::from_value(value)),
        // ast-grep-ignore: no-long-names
        Record::OrganizationMemberDeleteRequest => {
            // ast-grep-ignore: no-long-names
            recorded(members::OrganizationMemberDeleteRequest::from_value(value))
        }
        Record::TeamMemberUpdateRequest => recorded(TeamMemberUpdateRequest::from_value(value)),
        // ast-grep-ignore: no-long-names
        Record::OrganizationMemberUpdateRequest => {
            // ast-grep-ignore: no-long-names
            recorded(members::OrganizationMemberUpdateRequest::from_value(value))
        }
        Record::OrganizationTableUpdate => recorded(OrganizationTableUpdate::from_value(value)),
        Record::OrganizationUpdateRequestV2 => {
            patched(OrganizationUpdateRequestV2::from_value(value))
        }
        Record::OrganizationTableWithMembers => {
            recorded(OrganizationTableWithMembers::from_value(value))
        }
    }
}

fn recorded<T>(record: Result<T, MemberContractError>) -> HelperResult
where
    T: Serialize,
{
    let record = record.map_err(validation_failure)?;
    Ok(json!({
        "outcome": "record",
        "dump": record,
    }))
}

fn patched(request: Result<OrganizationUpdateRequestV2, MemberContractError>) -> HelperResult {
    let request = request.map_err(validation_failure)?;
    Ok(json!({
        "outcome": "record",
        "dump": request,
        "fields_set": request.fields_set(),
    }))
}

fn validation_failure(error: MemberContractError) -> Box<HelperError> {
    match error {
        MemberContractError::Validation { message } => Box::new(HelperError {
            class: "ValidationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        }),
        MemberContractError::Decode { source } => Box::<HelperError>::from(source),
    }
}
