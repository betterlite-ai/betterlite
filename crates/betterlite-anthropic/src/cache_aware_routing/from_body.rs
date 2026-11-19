use betterlite_models::decisions::Json;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_plan::parse_prompt;

impl AnthropicCacheRouting {
    #[must_use]
    pub fn from_body(body: &Json) -> Option<Self> {
        let text = serde_json::to_string(body).ok()?;
        let prefix = parse_prompt(&text)?;
        let limit = positive_integer(body, "max_tokens")?;
        Some(Self {
            body: body.clone(),
            prefix,
            requested_output_limit: limit,
        })
    }
}

fn positive_integer(body: &Json, field: &str) -> Option<i64> {
    let Json::Object(fields) = body else {
        return None;
    };
    match fields.get(field) {
        Some(Json::Number(number)) => number.as_i64().filter(|limit| *limit > 0),
        _ => None,
    }
}

mod tests;
