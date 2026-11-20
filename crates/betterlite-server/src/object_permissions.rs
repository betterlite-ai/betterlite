pub mod ambiguity;
pub mod attach;
pub mod cache;
pub mod entries;
pub mod error;
pub mod prepare;
pub mod update;

mod store;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::safe_json::safe_json_loads;

use crate::key_fields::decode::TypedValue;
use crate::key_fields::error::DecodeError;

// The outside-byte boundary: B192's loader bounds the bytes at 1 MiB and the JSON at 64 levels first.
pub fn decode_object_permission(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    let loaded = safe_json_loads(bytes)?;
    let json = serde_json::from_value::<JsonData>(loaded).map_err(DecodeError::Document)?;
    Ok(TypedValue { json })
}
