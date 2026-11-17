use std::fmt::Display;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{MemberContractError, field, json_object, missing_field};

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

fn member_list(object: &Map<String, Value>) -> Result<OneOrMany<Member>, MemberContractError> {
    let Some(raw) = object.get("member") else {
        return Err(missing_field("member"));
    };
    one_or_many(raw, record_from_json::<Member>)
}

fn org_member_list(
    object: &Map<String, Value>,
) -> Result<OneOrMany<OrgMember>, MemberContractError> {
    let Some(raw) = object.get("member") else {
        return Err(missing_field("member"));
    };
    one_or_many(raw, OrgMember::from_value)
}

fn org_role(object: &Map<String, Value>) -> Result<UserRole, MemberContractError> {
    let role: UserRole = field(object, "role", Value::Null)?;
    if ROLES_WITHIN_ORG.contains(&role) {
        return Ok(role);
    }
    Err(MemberContractError::Validation {
        message: "Input should be 'org_admin', 'internal_user' or 'internal_user_viewer'"
            .to_owned(),
    })
}

fn checked_role(role: Option<UserRole>) -> Result<Option<UserRole>, MemberContractError> {
    if role.is_some_and(|value| !ROLES_WITHIN_ORG.contains(&value)) {
        return Err(invalid_role());
    }
    Ok(role)
}

fn one_or_many<T>(
    raw: &Value,
    one: fn(&Value) -> Result<T, MemberContractError>,
) -> Result<OneOrMany<T>, MemberContractError> {
    if let Value::Array(items) = raw {
        return items
            .iter()
            .map(one)
            .collect::<Result<Vec<T>, MemberContractError>>()
            .map(OneOrMany::Many);
    }
    one(raw).map(OneOrMany::One)
}

fn record_from_json<T>(value: &Value) -> Result<T, MemberContractError>
where
    T: TryFrom<JsonData>,
    T::Error: Display,
{
    let data: JsonData = serde_json::from_value(value.clone())?;
    T::try_from(data).map_err(|source| MemberContractError::Validation {
        message: source.to_string(),
    })
}

fn member_identity(object: &Map<String, Value>) -> Result<MemberIdentity, MemberContractError> {
    let user_id: Option<String> = field(object, "user_id", Value::Null)?;
    let user_email: Option<String> = field(object, "user_email", Value::Null)?;
    MemberIdentity::new(user_id, user_email).map_err(|source| MemberContractError::Validation {
        message: source.to_string(),
    })
}

fn temp_budget_increase(
    object: &Map<String, Value>,
) -> Result<Option<Decimal>, MemberContractError> {
    let increase: Option<Decimal> = field(object, "temp_budget_increase", Value::Null)?;
    if increase.is_some_and(|amount| amount < Decimal::ZERO) {
        return Err(MemberContractError::Validation {
            message: "Input should be greater than or equal to 0".to_owned(),
        });
    }
    Ok(increase)
}

fn validate_temp_budget(request: &TeamMemberUpdateRequest) -> Result<(), MemberContractError> {
    match (request.temp_budget_increase, request.temp_budget_expiry) {
        (None, None) | (Some(_), Some(_)) => Ok(()),
        _ => Err(MemberContractError::Validation {
            message: "temp_budget_increase and temp_budget_expiry must be set together".to_owned(),
        }),
    }
}

fn invalid_role() -> MemberContractError {
    let quoted: Vec<String> = ROLES_WITHIN_ORG
        .iter()
        .map(|role| format!("'{role}'"))
        .collect();
    let listed = quoted.join(", ");
    MemberContractError::Validation {
        message: format!("Invalid role. Must be one of: [{listed}]"),
    }
}

mod tests;
