use std::fmt::Display;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use betterlite_db::{budget_permission_rows::JsonData, team_rows::member::Member};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_request_contracts::TeamContractError;
use betterlite_server::team_request_contracts::bulk_add::{
    BulkTeamMemberAddRequest, BulkTeamMemberAddResponse,
};
use betterlite_server::team_request_contracts::bulk_members::{
    BulkBudgetUpdateRequest, BulkTeamMemberDeleteRequest, TeamMemberBudgetPatch,
    TeamMemberBudgetUpdateResult, TeamMemberDeleteResult, TeamMemberRef,
};
use betterlite_server::team_request_contracts::create::NewTeamRequest;
use betterlite_server::team_request_contracts::membership::TeamMemberInfoResponse;
use betterlite_server::team_request_contracts::metadata::{
    TeamMetadataFieldSchema, TeamMetadataSchemaResponse,
};
use betterlite_server::team_request_contracts::permissions::{
    BulkPermissionUpdateRequest, BulkPermissionUpdateResponse, PermissionGetRequest,
    PermissionGetResponse, PermissionUpdateRequest,
};
use betterlite_server::team_request_contracts::shared::{RequestProjection, decode_request};
use betterlite_server::team_request_contracts::spend::{TeamUserSpendResponse, TeamUserSpendRow};
use betterlite_server::team_request_contracts::team_list::{TeamListEntry, TeamListResponse};
use betterlite_server::team_request_contracts::update::{PatchTeamRequest, UpdateTeamRequest};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    document: JsonData,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Create,
    Update,
    Patch,
    MemberRef,
    BudgetPatch,
    BulkDelete,
    BulkUpdate,
    PermissionGet,
    PermissionUpdate,
    BulkPermissions,
    PermissionGetResponse,
    BulkPermissionsResponse,
    TeamList,
    MemberInfo,
    MetadataField,
    MetadataSchema,
    BulkAdd,
    BulkAddResponse,
    SpendRow,
    SpendResponse,
    DeleteResult,
    BudgetUpdateResult,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let database = TimeZoneDatabase::from_env();
        let document = &input.document;
        match input.operation {
            Operation::Create => supplied(NewTeamRequest::from_value(document)),
            Operation::Update => supplied(UpdateTeamRequest::from_value(document)),
            Operation::Patch => supplied(PatchTeamRequest::from_value(document)),
            Operation::MemberRef => reference_answer(document),
            Operation::BudgetPatch => budget_patch_answer(document, &database),
            Operation::BulkDelete => answered(BulkTeamMemberDeleteRequest::from_value(document)),
            Operation::BulkUpdate => bulk_update_answer(document, &database),
            Operation::PermissionGet => answered(decode_request::<PermissionGetRequest>(document)),
            Operation::PermissionUpdate => {
                answered(decode_request::<PermissionUpdateRequest>(document))
            }
            Operation::BulkPermissions => {
                answered(decode_request::<BulkPermissionUpdateRequest>(document))
            }
            Operation::PermissionGetResponse => {
                answered(decode_request::<PermissionGetResponse>(document))
            }
            Operation::BulkPermissionsResponse => {
                answered(decode_request::<BulkPermissionUpdateResponse>(document))
            }
            Operation::TeamList => team_list_answer(document),
            Operation::MemberInfo => member_info_answer(document),
            Operation::MetadataField => answered(TeamMetadataFieldSchema::from_value(document)),
            Operation::MetadataSchema => answered(TeamMetadataSchemaResponse::from_value(document)),
            Operation::BulkAdd => bulk_add_answer(document),
            Operation::BulkAddResponse => {
                answered(decode_request::<BulkTeamMemberAddResponse>(document))
            }
            Operation::SpendRow => answered(decode_request::<TeamUserSpendRow>(document)),
            Operation::SpendResponse => answered(decode_request::<TeamUserSpendResponse>(document)),
            Operation::DeleteResult => answered(decode_request::<TeamMemberDeleteResult>(document)),
            Operation::BudgetUpdateResult => {
                answered(decode_request::<TeamMemberBudgetUpdateResult>(document))
            }
        }
    }
}

fn invalid<E>(message: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ValidationError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_string(),
    })
}

fn answered<T: Serialize>(built: Result<T, TeamContractError>) -> HelperResult {
    let value = built.map_err(invalid)?;
    let output = serde_json::to_value(value)?;
    Ok(output)
}

fn supplied<T: RequestProjection>(built: Result<T, TeamContractError>) -> HelperResult {
    let request = built.map_err(invalid)?;
    let projection = request.supplied().map_err(invalid)?;
    let output = serde_json::to_value(&projection)?;
    Ok(output)
}

const fn clocks() -> ResetClocks<'static> {
    let now: DateTime<Utc> = DateTime::UNIX_EPOCH;
    ResetClocks {
        reset_now: now,
        calendar_now: now,
        calendar_timezone: None,
    }
}

fn reference_answer(document: &JsonData) -> HelperResult {
    let reference = TeamMemberRef::from_value(document).map_err(invalid)?;
    Ok(json!({
        "user_id": reference.user_id,
        "user_email": reference.user_email,
    }))
}

fn budget_patch_answer(document: &JsonData, database: &TimeZoneDatabase) -> HelperResult {
    let patch = TeamMemberBudgetPatch::from_value(
        document,
        &BudgetResetSettings::default(),
        clocks(),
        database,
    )
    .map_err(invalid)?;
    let supplied_fields = serde_json::to_value(&patch.supplied_fields)?;
    let dump = serde_json::to_value(&patch)?;
    Ok(json!({ "supplied_fields": supplied_fields, "dump": dump }))
}

fn bulk_update_answer(document: &JsonData, database: &TimeZoneDatabase) -> HelperResult {
    answered(BulkBudgetUpdateRequest::from_value(
        document,
        &BudgetResetSettings::default(),
        clocks(),
        database,
    ))
}

fn team_list_answer(document: &JsonData) -> HelperResult {
    let response = TeamListResponse::from_value(document).map_err(invalid)?;
    Ok(json!({
        "teams": response.teams.iter().map(entry_answer).collect::<Vec<Value>>(),
        "total": response.total, "page": response.page,
        "page_size": response.page_size, "total_pages": response.total_pages, }))
}

fn entry_answer(entry: &TeamListEntry) -> Value {
    match entry {
        TeamListEntry::Enriched(item) => json!({
            "kind": "enriched", "team_id": item.team.fields.base.team_id,
            "members_count": item.members_count, "keys_count": item.keys_count,
            "access_group_models": item.access_group_models,
            "access_group_mcp_server_ids": item.access_group_mcp_server_ids,
            "access_group_agent_ids": item.access_group_agent_ids,
        }),
        TeamListEntry::Live(team) => json!({ "kind": "live", "team_id": team.fields.base.team_id }),
        TeamListEntry::Deleted(team) => json!({
            "kind": "deleted", "team_id": team.team.fields.base.team_id,
            "deleted_at": team.audit.deleted_at, "deleted_by": team.audit.deleted_by,
            "id": team.audit.id,
        }),
    }
}

fn member_info_answer(document: &JsonData) -> HelperResult {
    let info = decode_request::<TeamMemberInfoResponse>(document).map_err(invalid)?;
    Ok(json!({
        "user_id": info.membership.user_id, "team_id": info.membership.team_id,
        "budget_id": info.membership.budget_id, "role": info.role,
        "user_email": info.user_email, "team_alias": info.team_alias, }))
}

fn bulk_add_answer(document: &JsonData) -> HelperResult {
    let request = decode_request::<BulkTeamMemberAddRequest>(document).map_err(invalid)?;
    let members = request
        .members
        .as_ref()
        .map(|list| list.iter().map(member_answer).collect::<Vec<Value>>());
    Ok(json!({
        "team_id": request.team_id, "members": members,
        "all_users": request.all_users, "max_budget_in_team": request.max_budget_in_team,
    }))
}
fn member_answer(member: &Member) -> Value {
    json!({
        "user_id": member.base.user_id,
        "user_email": member.base.user_email,
        "role": member.role,
    })
}
