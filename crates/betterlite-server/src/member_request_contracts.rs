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

#[must_use]
pub fn decode_all(value: &Value) -> usize {
    let accepted = [
        team::DeleteTeamRequest::from_value(value).is_ok(),
        callbacks::AddTeamCallback::from_value(value).is_ok(),
        callbacks::TeamCallbackMetadata::from_value(value).is_ok(),
        members::OrgMember::from_value(value).is_ok(),
        members::MemberAddRequest::from_value(value).is_ok(),
        members::TeamMemberAddRequest::from_value(value).is_ok(),
        members::OrgMemberAddRequest::from_value(value).is_ok(),
        members::OrganizationMemberAddRequest::from_value(value).is_ok(),
        // ast-grep-ignore: no-long-names
        members::OrganizationMemberDeleteRequest::from_value(value).is_ok(),
        members::TeamMemberDeleteRequest::from_value(value).is_ok(),
        members::TeamMemberUpdateRequest::from_value(value).is_ok(),
        // ast-grep-ignore: no-long-names
        members::OrganizationMemberUpdateRequest::from_value(value).is_ok(),
        organization::OrganizationTableUpdate::from_value(value).is_ok(),
        organization::OrganizationUpdateRequestV2::from_value(value).is_ok(),
        organization::OrganizationTableWithMembers::from_value(value).is_ok(),
    ];
    accepted.iter().filter(|ok| **ok).count()
}
