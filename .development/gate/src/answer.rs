use serde::de::DeserializeOwned;

use crate::error::GateError;

pub(crate) fn parse_answer<Answer>(text: &str, what: &str) -> Result<Answer, GateError>
where
    Answer: DeserializeOwned,
{
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str(unfenced.trim()).map_err(|source| GateError::Json {
        what: what.to_owned(),
        source,
    })
}
