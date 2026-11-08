use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::usage::Usage;

// litellm.integrations.langfuse._extract_cache_read_input_tokens: the Anthropic top-level field, then a positive Gemini/OpenAI cached_tokens overriding it
#[must_use]
pub fn cache_read_tokens(usage: &Usage) -> i64 {
    let top_level = usage
        .extras
        .get("cache_read_input_tokens")
        .and_then(number_i64)
        .unwrap_or(0);
    let Some(details) = &usage.prompt_tokens_details else {
        return top_level;
    };
    let Some(Json::Number(cached_tokens)) = details.get("cached_tokens") else {
        return top_level;
    };
    let Ok(amount) = Decimal::from_str(&cached_tokens.to_string()) else {
        return top_level;
    };
    if amount <= Decimal::ZERO {
        return top_level;
    }
    amount.to_i64().unwrap_or(top_level)
}

fn number_i64(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => {
            let amount = Decimal::from_str(&number.to_string()).ok()?;
            amount.to_i64()
        }
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
