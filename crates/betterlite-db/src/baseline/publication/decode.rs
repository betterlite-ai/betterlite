use betterlite_models::baseline_history::model::BaselineHistory;
use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use betterlite_models::savings_evidence::numeric::round_decimal_lexeme;
use betterlite_models::savings_pricing::compare::ComparisonProvenance;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use super::model::{BaselinePublication, PublicationStatus};

const VERSION: i64 = 3;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPublication {
    #[serde(default = "version")]
    version: i64,
    comparison_id: String,
    comparison_started_at: f64,
    status: RawStatus,
    reason: String,
    #[serde(default)]
    provenance: Option<ComparisonProvenance>,
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawStatus {
    Estimated,
    Unknown,
}

const fn version() -> i64 {
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
        comparison_id: raw.comparison_id,
        comparison_started_at: raw.comparison_started_at,
        status: status(raw.status),
        reason: raw.reason,
        provenance: raw.provenance,
        actual_spend: raw.actual_spend.map(amount).transpose()?,
        baseline_spend: raw.baseline_spend.map(amount).transpose()?,
        input_tokens: raw.input_tokens,
        cache_read_input_tokens: raw.cache_read_input_tokens,
        cache_creation_5m_input_tokens: raw.cache_creation_5m_input_tokens,
        cache_creation_1h_input_tokens: raw.cache_creation_1h_input_tokens,
    })
}

const fn status(value: RawStatus) -> PublicationStatus {
    match value {
        RawStatus::Estimated => PublicationStatus::Estimated,
        RawStatus::Unknown => PublicationStatus::Unknown,
    }
}

fn amount(value: Number) -> Result<Decimal, PublicationDecodeError> {
    round_decimal_lexeme(&value.to_string()).map_err(PublicationDecodeError::Amount)
}

// BaselineHistory as the source adapter reads it: the history dataclass's own coercions apply.
pub fn decode_baseline_history(data: &[u8]) -> Result<BaselineHistory, serde_json::Error> {
    serde_json::from_slice(data)
}
