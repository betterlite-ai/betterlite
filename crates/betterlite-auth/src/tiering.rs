use betterlite_models::decisions::Json;
use betterlite_models::tiered_pricing::Tier;
use serde_json::Error;

// LiteLLM ignores a missing, null or non-list tiered_pricing table, but a malformed tier fails the estimate.
#[derive(Debug, thiserror::Error)]
pub enum TieringError {
    #[error("the tiered pricing table could not be decoded: {source}")]
    Decode {
        #[source]
        source: Error,
    },
}

pub(crate) fn pricing_tiers(table: Option<&Json>) -> Result<Option<Vec<Tier>>, TieringError> {
    let Some(Json::List(tiers)) = table else {
        return Ok(None);
    };
    if tiers.is_empty() {
        return Ok(None);
    }
    // B1666's Tier decodes from its own JSON shape, so the model map's table round-trips through it.
    let decoded =
        serde_json::to_string(tiers).and_then(|text| serde_json::from_str::<Vec<Tier>>(&text));
    decoded
        .map(Some)
        .map_err(|source| TieringError::Decode { source })
}
