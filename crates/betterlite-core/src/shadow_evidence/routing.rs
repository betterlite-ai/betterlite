use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::chat_text;
use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::responses_tool_namespaces::values::has_value;

mod tests;

#[must_use]
pub fn decision(metadata: &Json) -> IndexMap<String, Json> {
    match chat_text::field(metadata, "routing_decision") {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}

#[must_use]
pub fn routed_tier(metadata: &Json) -> Option<String> {
    let fields = decision(metadata);
    let raw = match fields.get("tier_label").filter(|value| has_value(value)) {
        Some(label) => label,
        None => fields.get("tier")?,
    };
    match raw {
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Some(string_value(raw))
        }
    }
}

#[must_use]
pub fn classifier_cost(metadata: &Json) -> Decimal {
    let fields = decision(metadata);
    match fields.get("classifier_cost") {
        Some(Json::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).unwrap_or(Decimal::ZERO)
        }
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => Decimal::ZERO,
    }
}
