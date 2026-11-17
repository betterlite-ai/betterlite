use std::fmt::Display;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{MemberContractError, json_object, optional_field, required_field};

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
        let role: UserRole = required_field(object, "role")?;
        if !ROLES_WITHIN_ORG.contains(&role) {
            return Err(MemberContractError::Validation {
                message: "Input should be 'org_admin', 'internal_user' or 'internal_user_viewer'"
                    .to_owned(),
            });
        }
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
        let raw: Value = required_field(object, "member")?;
        Ok(Self {
            member: one_or_many(&raw, record_from_json::<Member>)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgMemberAddRequest {
    pub member: OneOrMany<OrgMember>,
}

impl OrgMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let raw: Value = required_field(object, "member")?;
        Ok(Self {
            member: one_or_many(&raw, OrgMember::from_value)?,
        })
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

impl TeamMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let raw: Value = required_field(object, "member")?;
        Ok(Self {
            member: one_or_many(&raw, record_from_json::<Member>)?,
            team_id: required_field(object, "team_id")?,
            max_budget_in_team: optional_field(object, "max_budget_in_team")?,
            budget_duration: optional_field(object, "budget_duration")?,
            allowed_models: optional_field(object, "allowed_models")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationMemberAddRequest {
    pub member: OneOrMany<OrgMember>,
    pub organization_id: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget_in_organization: Option<Decimal>,
}

impl OrganizationMemberAddRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let raw: Value = required_field(object, "member")?;
        Ok(Self {
            member: one_or_many(&raw, OrgMember::from_value)?,
            organization_id: required_field(object, "organization_id")?,
            max_budget_in_organization: optional_field(object, "max_budget_in_organization")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMemberDeleteRequest {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub team_id: String,
}

impl TeamMemberDeleteRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let identity = member_identity(object)?;
        Ok(Self {
            user_id: identity.user_id,
            user_email: identity.user_email,
            team_id: required_field(object, "team_id")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
        let identity = member_identity(object)?;
        Ok(Self {
            user_id: identity.user_id,
            user_email: identity.user_email,
            organization_id: required_field(object, "organization_id")?,
        })
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

impl TeamMemberUpdateRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let request = Self {
            base: TeamMemberDeleteRequest::from_value(value)?,
            max_budget_in_team: optional_field(object, "max_budget_in_team")?,
            role: optional_field(object, "role")?,
            tpm_limit: optional_field(object, "tpm_limit")?,
            rpm_limit: optional_field(object, "rpm_limit")?,
            budget_duration: optional_field(object, "budget_duration")?,
            allowed_models: optional_field(object, "allowed_models")?,
            temp_budget_increase: temp_budget_increase(object)?,
            temp_budget_expiry: optional_field(object, "temp_budget_expiry")?,
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

// ast-grep-ignore: no-long-names
impl OrganizationMemberUpdateRequest {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        // ast-grep-ignore: no-long-names
        let base = OrganizationMemberDeleteRequest::from_value(value)?;
        let budget = optional_field(object, "max_budget_in_organization")?;
        let role: Option<UserRole> = optional_field(object, "role")?;
        if role.is_some_and(|role| !ROLES_WITHIN_ORG.contains(&role)) {
            return Err(invalid_role());
        }
        Ok(Self {
            base,
            max_budget_in_organization: budget,
            role,
        })
    }
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
    let user_id = optional_field(object, "user_id")?;
    let user_email = optional_field(object, "user_email")?;
    MemberIdentity::new(user_id, user_email).map_err(|source| MemberContractError::Validation {
        message: source.to_string(),
    })
}

fn temp_budget_increase(
    object: &Map<String, Value>,
) -> Result<Option<Decimal>, MemberContractError> {
    let increase: Option<Decimal> = optional_field(object, "temp_budget_increase")?;
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
