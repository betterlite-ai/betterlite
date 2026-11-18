use serde::{Serialize, de::DeserializeOwned};

use super::error::ProcessStreamError;

// The assembled response and the guardrail's answers cross between JSON and the typed guardrail value.
pub(super) fn convert<Source, Target>(
    value: Source,
    target: &'static str,
) -> Result<Target, ProcessStreamError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|source| ProcessStreamError::Convert { target, source })?;
    serde_json::from_value(json).map_err(|source| ProcessStreamError::Convert { target, source })
}
