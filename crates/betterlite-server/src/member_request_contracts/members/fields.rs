use std::fmt::Display;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::team_rows::member::Member;
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use super::{OneOrMany, OrgMember, ROLES_WITHIN_ORG, TeamMemberUpdateRequest};
use crate::member_request_contracts::{MemberContractError, field, missing_field};

pub(super) fn member_list(
    object: &Map<String, Value>,
) -> Result<OneOrMany<Member>, MemberContractError> {
    member_field(object, record_from_json::<Member>)
}

pub(super) fn org_member_list(
    object: &Map<String, Value>,
) -> Result<OneOrMany<OrgMember>, MemberContractError> {
    member_field(object, OrgMember::from_value)
}

pub(super) fn org_role(object: &Map<String, Value>) -> Result<UserRole, MemberContractError> {
    let role: UserRole = field(object, "role", Value::Null)?;
    if ROLES_WITHIN_ORG.contains(&role) {
        return Ok(role);
    }
    Err(MemberContractError::Validation {
        message: "Input should be 'org_admin', 'internal_user' or 'internal_user_viewer'"
            .to_owned(),
    })
}

pub(super) fn checked_role(
    role: Option<UserRole>,
) -> Result<Option<UserRole>, MemberContractError> {
    if role.is_some_and(|value| !ROLES_WITHIN_ORG.contains(&value)) {
        return Err(invalid_role());
    }
    Ok(role)
}

fn member_field<T>(
    object: &Map<String, Value>,
    one: fn(&Value) -> Result<T, MemberContractError>,
) -> Result<OneOrMany<T>, MemberContractError> {
    let Some(raw) = object.get("member") else {
        return Err(missing_field("member"));
    };
    let Value::Array(items) = raw else {
        return one(raw).map(OneOrMany::One);
    };
    items
        .iter()
        .map(one)
        .collect::<Result<Vec<T>, MemberContractError>>()
        .map(OneOrMany::Many)
}

pub(super) fn record_from_json<T>(value: &Value) -> Result<T, MemberContractError>
where
    T: TryFrom<JsonData>,
    T::Error: Display,
{
    let data: JsonData = serde_json::from_value(value.clone())?;
    T::try_from(data).map_err(|source| MemberContractError::Validation {
        message: source.to_string(),
    })
}

pub(super) fn member_identity(
    object: &Map<String, Value>,
) -> Result<MemberIdentity, MemberContractError> {
    let user_id: Option<String> = field(object, "user_id", Value::Null)?;
    let user_email: Option<String> = field(object, "user_email", Value::Null)?;
    MemberIdentity::new(user_id, user_email).map_err(|source| MemberContractError::Validation {
        message: source.to_string(),
    })
}

pub(super) fn temp_budget_increase(
    object: &Map<String, Value>,
) -> Result<Option<Decimal>, MemberContractError> {
    let increase: Option<Decimal> = field(object, "temp_budget_increase", Value::Null)?;
    match increase {
        Some(amount) if amount < Decimal::ZERO => Err(MemberContractError::Validation {
            message: "Input should be greater than or equal to 0".to_owned(),
        }),
        amount => Ok(amount),
    }
}

pub(super) fn validate_temp_budget(
    request: &TeamMemberUpdateRequest,
) -> Result<(), MemberContractError> {
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
