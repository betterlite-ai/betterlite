#![expect(
    clippy::disallowed_types,
    reason = "the records decode LiteLLM's request bodies from the raw JSON object the route received, before any typed field decoding"
)]

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub mod callbacks;
pub mod members;
pub mod organization;
pub mod team;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum MemberContractError {
    #[error("{message}")]
    Validation { message: String },
    #[error(transparent)]
    Decode { source: serde_json::Error },
}

pub(crate) fn json_object(value: &Value) -> Result<&Map<String, Value>, MemberContractError> {
    let Value::Object(object) = value else {
        return Err(MemberContractError::Validation {
            message: "Input should be a valid dictionary or object to extract fields from"
                .to_owned(),
        });
    };
    Ok(object)
}

impl From<serde_json::Error> for MemberContractError {
    fn from(source: serde_json::Error) -> Self {
        Self::Decode { source }
    }
}

pub(crate) fn required_field<T>(
    object: &Map<String, Value>,
    name: &str,
) -> Result<T, MemberContractError>
where
    T: DeserializeOwned,
{
    match object.get(name) {
        Some(raw) => Ok(serde_json::from_value(raw.clone())?),
        None => Ok(serde_json::from_value(Value::Null)?),
    }
}

pub(crate) fn optional_field<T>(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<T>, MemberContractError>
where
    T: DeserializeOwned,
{
    let Some(raw) = object.get(name) else {
        return Ok(None);
    };
    Ok(serde_json::from_value(raw.clone())?)
}

pub(crate) fn defaulted_field<T>(
    object: &Map<String, Value>,
    name: &str,
) -> Result<T, MemberContractError>
where
    T: DeserializeOwned + Default,
{
    let Some(raw) = object.get(name) else {
        return Ok(T::default());
    };
    Ok(serde_json::from_value(raw.clone())?)
}

pub fn decode_all(value: &Value) {
    drop(team::DeleteTeamRequest::from_value(value));
    drop(callbacks::AddTeamCallback::from_value(value));
    drop(callbacks::TeamCallbackMetadata::from_value(value));
    drop(members::OrgMember::from_value(value));
    drop(members::MemberAddRequest::from_value(value));
    drop(members::TeamMemberAddRequest::from_value(value));
    drop(members::OrgMemberAddRequest::from_value(value));
    drop(members::OrganizationMemberAddRequest::from_value(value));
    // ast-grep-ignore: no-long-names
    drop(members::OrganizationMemberDeleteRequest::from_value(value));
    drop(members::TeamMemberDeleteRequest::from_value(value));
    drop(members::TeamMemberUpdateRequest::from_value(value));
    // ast-grep-ignore: no-long-names
    drop(members::OrganizationMemberUpdateRequest::from_value(value));
    drop(organization::OrganizationTableUpdate::from_value(value));
    drop(organization::OrganizationUpdateRequestV2::from_value(value));
    drop(organization::OrganizationTableWithMembers::from_value(
        value,
    ));
}
