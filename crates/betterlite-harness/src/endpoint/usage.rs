use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use crate::types::Usage;

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
    pub const fn snapshot(&self) -> Usage {
        Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            calls: self.calls,
        }
    }
}

// A value of the wrong type is skipped instead of failing the whole payload.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Lenient<T> {
    Value(T),
    Other(IgnoredAny),
}

impl<T> Lenient<T> {
    pub(super) const fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Other(IgnoredAny) => None,
        }
    }
}

// Flattening this map makes serde read an object only, so a JSON array is not taken as positional fields.
type Rest = BTreeMap<String, Option<IgnoredAny>>;

// Missing is a counter the payload does not carry, so only it falls back to the other field name.
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Count {
    Number(Number),
    Unusable(IgnoredAny),
    #[default]
    #[serde(skip_deserializing)]
    Missing,
}

#[derive(Deserialize)]
pub struct TokenCounts {
    #[serde(default)]
    input_tokens: Count,
    #[serde(default)]
    prompt_tokens: Count,
    #[serde(default)]
    output_tokens: Count,
    #[serde(default)]
    completion_tokens: Count,
    #[serde(flatten)]
    _rest: Rest,
}

#[derive(Deserialize)]
pub struct Carrier {
    pub(super) usage: Option<Lenient<TokenCounts>>,
    #[serde(flatten)]
    _rest: Rest,
}

#[derive(Deserialize)]
pub struct Payload {
    #[serde(rename = "type")]
    pub(super) kind: Option<Lenient<String>>,
    pub(super) message: Option<Lenient<Carrier>>,
    pub(super) response: Option<Lenient<Carrier>>,
    pub(super) usage: Option<Lenient<TokenCounts>>,
    #[serde(flatten)]
    _rest: Rest,
}

#[must_use]
pub fn usage_from_mapping(usage: &TokenCounts) -> (u64, u64) {
    let input_tokens = token_field(&usage.input_tokens, &usage.prompt_tokens);
    let output_tokens = token_field(&usage.output_tokens, &usage.completion_tokens);
    (input_tokens, output_tokens)
}

#[must_use]
pub fn usage_from_body(body: &[u8]) -> (u64, u64) {
    let Ok(payload) = serde_json::from_slice::<Payload>(body) else {
        return (0, 0);
    };
    if let Some(usage) = payload.usage.as_ref().and_then(Lenient::value) {
        return usage_from_mapping(usage);
    }
    if let Some(response) = payload.response.as_ref().and_then(Lenient::value)
        && let Some(usage) = response.usage.as_ref().and_then(Lenient::value)
    {
        return usage_from_mapping(usage);
    }
    (0, 0)
}

// A present null does not fall back to the alternative field name, matching the source's dict.get
fn token_field(primary: &Count, fallback: &Count) -> u64 {
    let count = match primary {
        Count::Missing => fallback,
        Count::Number(_) | Count::Unusable(_) => primary,
    };
    let number = match count {
        Count::Number(number) => number,
        Count::Unusable(IgnoredAny) | Count::Missing => return 0,
    };
    if let Some(whole) = number.as_u64() {
        return whole;
    }
    let Some(float) = number.as_f64() else {
        return 0;
    };
    let Some(decimal) = Decimal::from_f64(float) else {
        return 0;
    };
    decimal.trunc().to_u64().unwrap_or(0)
}
