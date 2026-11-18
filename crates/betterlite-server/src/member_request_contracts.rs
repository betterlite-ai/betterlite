use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::de::Error as _;

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

pub(crate) fn json_object(value: &Json) -> Result<&IndexMap<String, Json>, MemberContractError> {
    let Json::Object(object) = value else {
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

pub(crate) fn missing_field(name: &str) -> MemberContractError {
    MemberContractError::Decode {
        source: serde_json::Error::custom(format!("missing field `{name}`")),
    }
}

pub(crate) fn field<T>(
    object: &IndexMap<String, Json>,
    name: &str,
    fallback: &Json,
) -> Result<T, MemberContractError>
where
    T: DeserializeOwned,
{
    let raw = object.get(name).unwrap_or(fallback);
    let text = serde_json::to_string(raw)?;
    Ok(serde_json::from_str(&text)?)
}

#[must_use]
pub fn decode_all(value: &Json) -> usize {
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
