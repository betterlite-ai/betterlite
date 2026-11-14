use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::Json;

use super::error::TransformError;

// serde already knows both shapes, so the decisions-to-prompts direction is its own round trip.
pub(crate) fn from_contract(value: &ContractJson) -> Result<Json, TransformError> {
    serde_json::to_string(value)
        .and_then(|encoded| serde_json::from_str(&encoded))
        .map_err(TransformError::from)
}
