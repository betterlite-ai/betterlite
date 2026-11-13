use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::types::{JsonObject, Usage};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageTracker {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: Decimal,
    pub calls: u64,
}

impl UsageTracker {
    pub fn add(&mut self, input_tokens: u64, output_tokens: u64, cost: Decimal) {
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.cost = self.cost.saturating_add(cost);
        self.calls = self.calls.saturating_add(1);
    }

    #[must_use]
    pub fn snapshot(&self) -> Usage {
        Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            calls: self.calls,
        }
    }
}

pub(super) fn child_object(parent: &JsonObject, key: &str) -> Option<JsonObject> {
    match parent.get(key) {
        Some(value) => serde_json::from_value(value.clone()).ok(),
        None => None,
    }
}

#[must_use]
pub fn usage_from_mapping(usage: &JsonObject) -> (u64, u64) {
    let input_tokens = token_field(usage, "input_tokens", "prompt_tokens");
    let output_tokens = token_field(usage, "output_tokens", "completion_tokens");
    (input_tokens, output_tokens)
}

#[must_use]
pub fn usage_from_body(body: &JsonObject) -> (u64, u64) {
    if let Some(usage) = child_object(body, "usage") {
        return usage_from_mapping(&usage);
    }
    if let Some(response) = child_object(body, "response")
        && let Some(usage) = child_object(&response, "usage")
    {
        return usage_from_mapping(&usage);
    }
    (0, 0)
}

// A present null does not fall back to the alternative field name, matching the source's dict.get
fn token_field(usage: &JsonObject, primary: &str, fallback: &str) -> u64 {
    let Some(value) = usage.get(primary).or_else(|| usage.get(fallback)) else {
        return 0;
    };
    if value.is_boolean() {
        return 0;
    }
    if let Some(count) = value.as_u64() {
        return count;
    }
    let Some(number) = value.as_f64() else {
        return 0;
    };
    let Some(number) = Decimal::from_f64(number) else {
        return 0;
    };
    number.trunc().to_u64().unwrap_or(0)
}
