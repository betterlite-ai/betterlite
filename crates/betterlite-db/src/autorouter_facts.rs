pub mod cache;
pub mod session;
pub mod turn;
pub mod turn_time;

use betterlite_models::decisions::Json;
use betterlite_models::savings_evidence::error::SavingsEvidenceError;

// INTERNAL_CALL_ORIGIN_METADATA_KEY of litellm/constants.py.
pub const INTERNAL_CALL_ORIGIN_KEY: &str = "internal_call_origin";

#[derive(Debug, thiserror::Error)]
pub enum AutorouterFactsError {
    #[error("the turn cache usage object could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the turn cache usage object could not be decoded")]
    Decode(#[source] serde_json::Error),
    #[error("the turn cache token count could not be read")]
    Cache(#[from] SavingsEvidenceError),
    #[error("the turn's token total overflows a 64-bit integer")]
    TokenTotalOverflow,
    #[error("the turn's spend overflows the decimal range")]
    SpendOverflow,
}

// The source tests `if value:` on metadata fields its types leave untyped, so one JSON value stands for that decision.
pub(crate) fn counts_as_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
