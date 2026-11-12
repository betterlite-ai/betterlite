use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::MIN_THINKING_BUDGET_TOKENS;

// Anthropic needs max_tokens > budget_tokens: None drops thinking, Some keeps or caps the caller's dict.
#[must_use]
pub fn cap_thinking_to_max_tokens(
    thinking: &IndexMap<String, Json>,
    max_tokens: Option<i64>,
) -> Option<Json> {
    let Some(limit) = max_tokens else {
        return Some(Json::Object(thinking.clone()));
    };
    let Some(budget) = integer_budget(thinking.get("budget_tokens")) else {
        return Some(Json::Object(thinking.clone()));
    };
    if limit <= MIN_THINKING_BUDGET_TOKENS {
        return None;
    }
    if budget < i128::from(limit) {
        return Some(Json::Object(thinking.clone()));
    }
    let mut capped = IndexMap::new();
    capped.insert(
        "type".to_owned(),
        thinking
            .get("type")
            .cloned()
            .unwrap_or_else(|| Json::Text("enabled".to_owned())),
    );
    capped.insert(
        "budget_tokens".to_owned(),
        Json::Number(Number::from(limit.saturating_sub(1))),
    );
    Some(Json::Object(capped))
}

// Python's isinstance(budget, int): JSON integers and booleans (True is 1, False is 0), nothing else.
fn integer_budget(value: Option<&Json>) -> Option<i128> {
    match value {
        Some(Json::Number(number)) => integer_number(number),
        Some(Json::Bool(flag)) => Some(i128::from(*flag)),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => None,
    }
}

// Arbitrary-precision numbers keep their digits, so an integer past i64 still compares as one.
fn integer_number(number: &Number) -> Option<i128> {
    if let Some(value) = number.as_i64() {
        return Some(i128::from(value));
    }
    if let Some(unsigned) = number.as_u64() {
        return Some(i128::from(unsigned));
    }
    number.to_string().parse::<i128>().ok()
}
