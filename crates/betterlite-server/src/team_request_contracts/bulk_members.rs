use std::collections::BTreeSet;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::{ResetClocks, budget_duration_error};
use betterlite_db::budget_permission_rows::JsonData;
use jiff::tz::TimeZoneDatabase;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;
use crate::management_envelopes::ResourceResponse;

use super::TeamContractError;
use super::shared::{
    MAX_BULK_BUDGET_UPDATES, MAX_BULK_TEAM_MEMBER_DELETES, check_member_count, decode_request,
    object,
};
use super::values::BudgetSource;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamMemberRef {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
}

impl TeamMemberRef {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let reference: Self = decode_request(value)?;
        require_identity(&reference)?;
        reject_both_identifiers(&reference)?;
        Ok(reference)
    }
}

fn require_identity(reference: &TeamMemberRef) -> Result<(), TeamContractError> {
    if reference.user_id.is_none() && reference.user_email.is_none() {
        return Err(TeamContractError::Validation {
            message: "Either user id or user email must be provided".to_owned(),
        });
    }
    Ok(())
}

fn reject_both_identifiers(reference: &TeamMemberRef) -> Result<(), TeamContractError> {
    if reference.user_id.is_some() && reference.user_email.is_some() {
        return Err(TeamContractError::Validation {
            message: "Each member must be identified by exactly one of user_id or user_email"
                .to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMemberBudgetPatch {
    #[serde(flatten)]
    pub reference: TeamMemberRef,
    pub max_budget_in_team: Option<Amount>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub budget_duration: Option<String>,
    pub allowed_models: Option<Vec<String>>,
    #[serde(skip)]
    pub supplied_fields: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamMemberPatchInput {
    user_id: Option<String>,
    user_email: Option<String>,
    max_budget_in_team: Option<Amount>,
    tpm_limit: Option<i64>,
    rpm_limit: Option<i64>,
    budget_duration: Option<String>,
    allowed_models: Option<Vec<String>>,
}

impl TeamMemberBudgetPatch {
    pub fn from_value(
        value: &JsonData,
        settings: &BudgetResetSettings,
        clocks: ResetClocks<'_>,
        database: &TimeZoneDatabase,
    ) -> Result<Self, TeamContractError> {
        let input: TeamMemberPatchInput = decode_request(value)?;
        let reference = TeamMemberRef {
            user_id: input.user_id,
            user_email: input.user_email,
        };
        require_identity(&reference)?;
        let duration =
            budget_duration_error(input.budget_duration.as_deref(), settings, clocks, database);
        if let Some(message) = duration {
            return Err(TeamContractError::Validation { message });
        }
        reject_both_identifiers(&reference)?;
        let supplied_fields: BTreeSet<String> = object(value)?.keys().cloned().collect();
        Ok(Self {
            reference,
            max_budget_in_team: input.max_budget_in_team,
            tpm_limit: input.tpm_limit,
            rpm_limit: input.rpm_limit,
            budget_duration: input.budget_duration,
            allowed_models: input.allowed_models,
            supplied_fields,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkTeamMemberDeleteRequest {
    pub members: Vec<TeamMemberRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkDeleteInput {
    members: Vec<JsonData>,
}

impl BulkTeamMemberDeleteRequest {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let input: BulkDeleteInput = decode_request(value)?;
        check_member_count(input.members.len(), MAX_BULK_TEAM_MEMBER_DELETES)?;
        let members = input
            .members
            .iter()
            .map(TeamMemberRef::from_value)
            .collect::<Result<Vec<TeamMemberRef>, TeamContractError>>()?;
        Ok(Self { members })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkBudgetUpdateRequest {
    pub members: Vec<TeamMemberBudgetPatch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkBudgetUpdateInput {
    members: Vec<JsonData>,
}

impl BulkBudgetUpdateRequest {
    pub fn from_value(
        value: &JsonData,
        settings: &BudgetResetSettings,
        clocks: ResetClocks<'_>,
        database: &TimeZoneDatabase,
    ) -> Result<Self, TeamContractError> {
        let input: BulkBudgetUpdateInput = decode_request(value)?;
        check_member_count(input.members.len(), MAX_BULK_BUDGET_UPDATES)?;
        let members = input
            .members
            .iter()
            .map(|member| TeamMemberBudgetPatch::from_value(member, settings, clocks, database))
            .collect::<Result<Vec<TeamMemberBudgetPatch>, TeamContractError>>()?;
        Ok(Self { members })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberDeleteResult {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberBudgetUpdateResult {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub budget_id: Option<String>,
    pub max_budget: Option<Amount>,
    pub max_budget_source: Option<BudgetSource>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub budget_duration: Option<String>,
    pub allowed_models: Option<Vec<String>>,
}

pub type BulkTeamMemberDeleteResponse = ResourceResponse<Vec<TeamMemberDeleteResult>>;

pub type BulkBudgetUpdateResponse = ResourceResponse<Vec<TeamMemberBudgetUpdateResult>>;
