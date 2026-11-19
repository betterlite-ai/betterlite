use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::decoder::TeamRowError;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use serde::Serialize;

use crate::key_fields::error::DecodeError as KeyDecodeError;
use crate::proxy_request_contracts::ContractError;

pub mod bulk_add;
pub mod bulk_members;
pub mod create;
pub mod membership;
pub mod metadata;
pub mod permissions;
pub mod shared;
pub mod spend;
pub mod team_list;
pub mod update;
pub mod values;

mod tests;

// The owned outside-byte boundary: the logging loader bounds the bytes at 1 MiB and the JSON at depth 64.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Bounded(#[from] PrivateJsonError),
    #[error("the team request document is not valid JSON")]
    Document(#[source] serde_json::Error),
    #[error("the team request document is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TypedValue {
    pub json: JsonData,
}

#[derive(Debug, thiserror::Error)]
pub enum TeamContractError {
    #[error("{message}")]
    Validation { message: String },
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Row(#[from] TeamRowError),
    #[error(transparent)]
    KeyDecode(#[from] KeyDecodeError),
}

pub fn decode_team_request(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    let loaded = safe_json_loads(bytes)?;
    let value = serde_json::from_value::<JsonData>(loaded).map_err(DecodeError::Document)?;
    let JsonData::Object(_) = value else {
        return Err(DecodeError::NotAnObject);
    };
    Ok(TypedValue { json: value })
}
