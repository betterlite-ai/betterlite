use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::safe_json::safe_json_loads;

use crate::proxy_request_contracts::ContractError;
use crate::user_contracts::error::UserContractsError;
use crate::user_contracts::query::user_id_from_query;

pub mod bulk_delete;
pub mod bulk_new;
pub mod bulk_update;
pub mod error;
pub mod list;
pub mod query;
pub mod search;
pub mod wire;

mod tests;

// The owned outside-byte boundary: the logging crate's loader bounds the bytes at 1 MiB and the JSON at depth 64.
pub fn decode_bulk_user_request<T>(
    bytes: &[u8],
    read: fn(&JsonValue) -> Result<T, ContractError>,
) -> Result<T, UserContractsError> {
    match safe_json_loads(bytes) {
        Ok(value) => read(&value).map_err(UserContractsError::from),
        Err(reason) => Err(UserContractsError::from(reason)),
    }
}

// LiteLLM recovers from the raw query only when the query-map value decoded a literal plus as a space.
pub fn decode_user_info_query(query: &str, supplied: Option<&str>) -> Option<String> {
    let Some(user_id) = supplied else {
        return None;
    };
    if user_id.contains(' ') {
        return user_id_from_query(query);
    }
    Some(user_id.to_owned())
}
