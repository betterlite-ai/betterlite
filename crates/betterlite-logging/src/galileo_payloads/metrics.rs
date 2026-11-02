use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use super::support::record_i64;

// litellm.integrations.galileo.GalileoObserve._token_metrics_from_record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenMetrics {
    pub num_input_tokens: i64,
    pub num_output_tokens: i64,
    pub num_total_tokens: i64,
    pub cost: Option<Decimal>,
}

#[must_use]
pub fn token_metrics_from_record(record: &IndexMap<String, Json>) -> TokenMetrics {
    let num_input_tokens = record_i64(record, "num_input_tokens");
    let num_output_tokens = record_i64(record, "num_output_tokens");
    let mut num_total_tokens = record_i64(record, "num_total_tokens");
    if num_total_tokens == 0 && (num_input_tokens != 0 || num_output_tokens != 0) {
        num_total_tokens = num_input_tokens.saturating_add(num_output_tokens);
    }
    let cost = match record.get("cost") {
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    TokenMetrics {
        num_input_tokens,
        num_output_tokens,
        num_total_tokens,
        cost,
    }
}

// The dict[str, object] _token_metrics_from_record returns, in its field order; cost is present only when the record carried one.
#[must_use]
pub fn token_metrics_json(metrics: TokenMetrics) -> IndexMap<String, Json> {
    let mut object = IndexMap::new();
    object.insert(
        "num_input_tokens".to_owned(),
        Json::Number(serde_json::Number::from(metrics.num_input_tokens)),
    );
    object.insert(
        "num_output_tokens".to_owned(),
        Json::Number(serde_json::Number::from(metrics.num_output_tokens)),
    );
    object.insert(
        "num_total_tokens".to_owned(),
        Json::Number(serde_json::Number::from(metrics.num_total_tokens)),
    );
    if let Some(cost) = metrics.cost
        && let Ok(number) = serde_json::Number::from_str(&cost.to_string())
    {
        object.insert("cost".to_owned(), Json::Number(number));
    }
    object
}
