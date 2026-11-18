use serde::{Serialize, de::DeserializeOwned};

use super::error::ProcessStreamError;

// The assembled response and the guardrail's answers cross between JSON and the typed guardrail value.
pub(super) fn convert<Source, Target>(value: Source) -> Result<Target, ProcessStreamError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    match serde_json::to_value(value) {
        Ok(encoded) => serde_json::from_value(encoded).map_err(ProcessStreamError::Convert),
        Err(error) => Err(ProcessStreamError::Convert(error)),
    }
}
