use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::fields::{
    checked_role, member_identity, member_list, org_member_list, org_role, record_from_json,
    temp_budget_increase, validate_temp_budget,
};
use super::{MemberContractError, json_object};

pub const ROLES_WITHIN_ORG: [UserRole; 3] = [
    UserRole::OrgAdmin,
    UserRole::InternalUser,
    UserRole::InternalUserViewer,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgMember {
    #[serde(flatten)]
    pub base: MemberBase,
    pub role: UserRole,
}

impl OrgMember {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let base: MemberBase = record_from_json(value)?;
        let object = json_object(value)?;
        let role = org_role(object)?;
        Ok(Self { base, role })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberAddRequest {
    pub member: OneOrMany<Member>,
}

impl MemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let member = member_list(object)?;
        Ok(Self { member })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgMemberAddRequest {
    pub member: OneOrMany<OrgMember>,
}

impl OrgMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let member = org_member_list(object)?;
        Ok(Self { member })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMemberAddRequest {
    pub member: OneOrMany<Member>,
    pub team_id: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget_in_team: Option<Decimal>,
    pub budget_duration: Option<String>,
    pub allowed_models: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TeamMemberAddFields {
    team_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    max_budget_in_team: Option<Decimal>,
    budget_duration: Option<String>,
    allowed_models: Option<Vec<String>>,
}

impl TeamMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let member = member_list(object)?;
        let fields: TeamMemberAddFields = serde_json::from_value(value.clone())?;
        Ok(Self::new(member, fields))
    }

    const fn new(member: OneOrMany<Member>, fields: TeamMemberAddFields) -> Self {
        Self {
            member,
            team_id: fields.team_id,
            max_budget_in_team: fields.max_budget_in_team,
            budget_duration: fields.budget_duration,
            allowed_models: fields.allowed_models,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationMemberAddRequest {
    pub member: OneOrMany<OrgMember>,
    pub organization_id: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget_in_organization: Option<Decimal>,
}

#[derive(Deserialize)]
struct OrganizationMemberAddFields {
    organization_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    max_budget_in_organization: Option<Decimal>,
}

impl OrganizationMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let member = org_member_list(object)?;
        let fields: OrganizationMemberAddFields = serde_json::from_value(value.clone())?;
        Ok(Self::new(member, fields))
    }

    const fn new(member: OneOrMany<OrgMember>, fields: OrganizationMemberAddFields) -> Self {
        Self {
            member,
            organization_id: fields.organization_id,
            max_budget_in_organization: fields.max_budget_in_organization,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberDeleteRequest {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub team_id: String,
}

impl TeamMemberDeleteRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        member_identity(object)?;
        let request: Self = serde_json::from_value(value.clone())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// ast-grep-ignore: no-long-names
pub struct OrganizationMemberDeleteRequest {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub organization_id: String,
}

// ast-grep-ignore: no-long-names
impl OrganizationMemberDeleteRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        member_identity(object)?;
        let request: Self = serde_json::from_value(value.clone())?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMemberUpdateRequest {
    #[serde(flatten)]
    pub base: TeamMemberDeleteRequest,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget_in_team: Option<Decimal>,
    pub role: Option<MemberRole>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub budget_duration: Option<String>,
    pub allowed_models: Option<Vec<String>>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub temp_budget_increase: Option<Decimal>,
    pub temp_budget_expiry: Option<Timestamp>,
}

#[derive(Deserialize)]
struct TeamMemberUpdateFields {
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    max_budget_in_team: Option<Decimal>,
    role: Option<MemberRole>,
    tpm_limit: Option<i64>,
    rpm_limit: Option<i64>,
    budget_duration: Option<String>,
    allowed_models: Option<Vec<String>>,
    temp_budget_expiry: Option<Timestamp>,
}

impl TeamMemberUpdateRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let base = TeamMemberDeleteRequest::from_value(value)?;
        let fields: TeamMemberUpdateFields = serde_json::from_value(value.clone())?;
        let increase = temp_budget_increase(object)?;
        Self::from_fields(base, fields, increase)
    }

    fn from_fields(
        base: TeamMemberDeleteRequest,
        fields: TeamMemberUpdateFields,
        increase: Option<Decimal>,
    ) -> Result<Self, MemberContractError> {
        let request = Self {
            base,
            max_budget_in_team: fields.max_budget_in_team,
            role: fields.role,
            tpm_limit: fields.tpm_limit,
            rpm_limit: fields.rpm_limit,
            budget_duration: fields.budget_duration,
            allowed_models: fields.allowed_models,
            temp_budget_increase: increase,
            temp_budget_expiry: fields.temp_budget_expiry,
        };
        validate_temp_budget(&request)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
// ast-grep-ignore: no-long-names
pub struct OrganizationMemberUpdateRequest {
    #[serde(flatten)]
    // ast-grep-ignore: no-long-names
    pub base: OrganizationMemberDeleteRequest,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget_in_organization: Option<Decimal>,
    pub role: Option<UserRole>,
}

#[derive(Deserialize)]
struct OrganizationMemberUpdateFields {
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    max_budget_in_organization: Option<Decimal>,
    role: Option<UserRole>,
}

// ast-grep-ignore: no-long-names
impl OrganizationMemberUpdateRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        // ast-grep-ignore: no-long-names
        let base = OrganizationMemberDeleteRequest::from_value(value)?;
        let fields: OrganizationMemberUpdateFields = serde_json::from_value(value.clone())?;
        Self::from_fields(base, fields)
    }

    fn from_fields(
        // ast-grep-ignore: no-long-names
        base: OrganizationMemberDeleteRequest,
        fields: OrganizationMemberUpdateFields,
    ) -> Result<Self, MemberContractError> {
        let role = checked_role(fields.role)?;
        Ok(Self {
            base,
            max_budget_in_organization: fields.max_budget_in_organization,
            role,
        })
    }
}

mod fields;
mod tests;
