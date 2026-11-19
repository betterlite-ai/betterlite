use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

const COMPRESSION_SAVINGS_SOURCE: &str = "compression_interception";
const LITELLM_METADATA_FIELD: &str = "litellm_metadata";
const COMPRESSION_SAVINGS_FIELD: &str = "compression_savings";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionSavings {
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub tokens_saved: u64,
}

#[must_use]
pub fn savings_from_counts(
    original_tokens: &Json,
    compressed_tokens: &Json,
) -> Option<CompressionSavings> {
    // A negative or a compressed count above the original has no unsigned form, so both fall out here
    let before = u64::try_from(token_count(original_tokens)?).ok()?;
    let after = u64::try_from(token_count(compressed_tokens)?).ok()?;
    Some(CompressionSavings {
        tokens_before: before,
        tokens_after: after,
        tokens_saved: before.checked_sub(after)?,
    })
}

pub fn record_compression_savings(
    request: &mut IndexMap<String, Json>,
    savings: &CompressionSavings,
) {
    let recorded = savings_json(savings);
    if let Some(Json::Object(metadata)) = request.get_mut(LITELLM_METADATA_FIELD) {
        metadata.insert(COMPRESSION_SAVINGS_FIELD.to_owned(), recorded);
        return;
    }
    let metadata = IndexMap::from([(COMPRESSION_SAVINGS_FIELD.to_owned(), recorded)]);
    request.insert(LITELLM_METADATA_FIELD.to_owned(), Json::Object(metadata));
}

// The source accepts int and rejects bool, float and anything else, so a JSON bool is not a count.
fn token_count(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn savings_json(savings: &CompressionSavings) -> Json {
    Json::Object(IndexMap::from([
        (
            "tokens_before".to_owned(),
            Json::Number(Number::from(savings.tokens_before)),
        ),
        (
            "tokens_after".to_owned(),
            Json::Number(Number::from(savings.tokens_after)),
        ),
        (
            "tokens_saved".to_owned(),
            Json::Number(Number::from(savings.tokens_saved)),
        ),
        (
            "source".to_owned(),
            Json::Text(COMPRESSION_SAVINGS_SOURCE.to_owned()),
        ),
    ]))
}
