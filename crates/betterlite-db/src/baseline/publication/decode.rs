use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_models::savings_evidence::numeric::round_decimal_lexeme;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use super::model::{BaselinePublication, PublicationProvenance, PublicationStatus, VERSION};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPublication {
    #[serde(default = "default_version")]
    version: i64,
    comparison_id: String,
    comparison_started_at: f64,
    status: PublicationStatus,
    reason: String,
    #[serde(default)]
    provenance: Option<PublicationProvenance>,
    #[serde(default)]
    actual_spend: Option<Number>,
    #[serde(default)]
    baseline_spend: Option<Number>,
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_5m_input_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_1h_input_tokens: Option<i64>,
}

const fn default_version() -> i64 {
    VERSION
}

#[derive(Debug, thiserror::Error)]
pub enum PublicationDecodeError {
    #[error("baseline publication is not valid JSON of the required shape")]
    Json(#[from] serde_json::Error),
    #[error("baseline publication version must be 3")]
    Version,
    #[error("baseline publication amount is not a finite number")]
    Amount(#[source] DecimalLexemeError),
}

// Stored publication text, validated the way the source model validates it, never coerced.
pub fn decode_baseline_publication(
    data: &[u8],
) -> Result<BaselinePublication, PublicationDecodeError> {
    let raw: RawPublication = serde_json::from_slice(data)?;
    if raw.version != VERSION {
        return Err(PublicationDecodeError::Version);
    }
    Ok(BaselinePublication {
        version: raw.version,
        comparison_id: raw.comparison_id,
        comparison_started_at: raw.comparison_started_at,
        status: raw.status,
        reason: raw.reason,
        provenance: raw.provenance,
        actual_spend: amount(raw.actual_spend)?,
        baseline_spend: amount(raw.baseline_spend)?,
        input_tokens: raw.input_tokens,
        cache_read_input_tokens: raw.cache_read_input_tokens,
        cache_creation_5m_input_tokens: raw.cache_creation_5m_input_tokens,
        cache_creation_1h_input_tokens: raw.cache_creation_1h_input_tokens,
    })
}

fn amount(value: Option<Number>) -> Result<Option<Decimal>, PublicationDecodeError> {
    value
        .map(|number| round_decimal_lexeme(&number.to_string()))
        .transpose()
        .map_err(PublicationDecodeError::Amount)
}
