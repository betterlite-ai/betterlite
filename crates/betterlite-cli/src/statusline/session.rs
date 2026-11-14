use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::value::RawValue;

use super::credentials::printable;

const BASELINE_KEY: &str = "baseline_spend";
const ESTIMATED_BASELINE_KEY: &str = "savings_estimated_baseline_spend";
const NULL: &str = "null";

// Field names are the ones LiteLLM's script writes into its cache, so both read each other's entries
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Session {
    pub router_name: String,
    pub last_model: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub baseline_spend: Option<Decimal>,
    pub baseline_model: Option<String>,
    pub turns: Option<u64>,
    pub savings_estimated_turns: Option<u64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub savings_estimated_actual_spend: Option<Decimal>,
}

impl Session {
    // The proxy's answer is trusted only once its labels and money fields pass; anything else is no session
    #[must_use]
    pub fn from_json(body: &[u8]) -> Option<Self> {
        let fields: BTreeMap<String, &RawValue> = serde_json::from_slice(body).ok()?;
        let router_name = text(&fields, "router_name");
        let last_model = text(&fields, "last_model");
        if router_name.is_empty() || last_model.is_empty() {
            return None;
        }
        let spend = decimal(fields.get("spend")?)?;
        // the estimated key wins whenever it is present, even as null
        let baseline = fields
            .get(ESTIMATED_BASELINE_KEY)
            .or_else(|| fields.get(BASELINE_KEY));
        let baseline_spend = match baseline {
            Some(raw) if raw.get() != NULL => Some(decimal(raw)?),
            _ => None,
        };
        Some(Self {
            router_name,
            last_model,
            spend,
            baseline_spend,
            baseline_model: Some(text(&fields, "baseline_model")).filter(|name| !name.is_empty()),
            turns: fields.get("turns").and_then(|raw| raw.get().parse().ok()),
            // present but invalid counts as zero estimated turns, absent or null as unknown
            savings_estimated_turns: fields
                .get("savings_estimated_turns")
                .filter(|raw| raw.get() != NULL)
                .map(|raw| raw.get().parse().unwrap_or(0)),
            savings_estimated_actual_spend: fields
                .get("savings_estimated_actual_spend")
                .and_then(|raw| decimal(*raw))
                .filter(|amount| *amount >= Decimal::ZERO),
        })
    }
}

fn text(fields: &BTreeMap<String, &RawValue>, key: &str) -> String {
    fields
        .get(key)
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .map_or_else(String::new, |value| printable(&value))
}

// Only a JSON number counts: a string, a boolean or null is not money, and one too large for Decimal is not finite
#[must_use]
pub fn decimal(raw: &RawValue) -> Option<Decimal> {
    let literal = raw.get();
    if !literal.starts_with(|first: char| first == '-' || first.is_ascii_digit()) {
        return None;
    }
    literal
        .parse::<Decimal>()
        .or_else(|_| Decimal::from_scientific(literal))
        .ok()
}

mod tests;
