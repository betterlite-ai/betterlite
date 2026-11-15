use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::credentials::printable;
use super::wire::{Wire, text};

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

// a present key counts even as null and hides the plain baseline, so a null stays apart from an absent key
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Estimated {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Present(Wire<Number>),
}

#[derive(Deserialize)]
struct Payload {
    router_name: Option<Wire<String>>,
    last_model: Option<Wire<String>>,
    spend: Option<Wire<Number>>,
    #[serde(rename = "savings_estimated_baseline_spend", default)]
    estimated_baseline: Estimated,
    baseline_spend: Option<Wire<Number>>,
    baseline_model: Option<Wire<String>>,
    turns: Option<Wire<Number>>,
    savings_estimated_turns: Option<Wire<Number>>,
    savings_estimated_actual_spend: Option<Wire<Number>>,
}
impl Session {
    // The proxy's answer is trusted only once its labels and money fields pass; anything else is no session
    pub(crate) fn from_json(body: &[u8]) -> Option<Self> {
        let payload: Payload = serde_json::from_slice(body).ok()?;
        let router_name = printable(text(payload.router_name.as_ref()));
        let last_model = printable(text(payload.last_model.as_ref()));
        if router_name.is_empty() || last_model.is_empty() {
            return None;
        }
        let spend = payload.spend.as_ref()?.amount()?;
        let baseline = match &payload.estimated_baseline {
            Estimated::Present(found) => Some(found),
            Estimated::Absent => payload.baseline_spend.as_ref(),
        };
        let baseline_spend = match baseline {
            None | Some(Wire::Null) => None,
            Some(found) => Some(found.amount()?),
        };
        Some(Self {
            router_name,
            last_model,
            spend,
            baseline_spend,
            baseline_model: Some(printable(text(payload.baseline_model.as_ref())))
                .filter(|name| !name.is_empty()),
            turns: payload.turns.as_ref().and_then(Wire::count),
            // present but invalid counts as zero estimated turns, absent or null as unknown
            savings_estimated_turns: match payload.savings_estimated_turns.as_ref() {
                None | Some(Wire::Null) => None,
                Some(found) => Some(found.count().unwrap_or(0)),
            },
            savings_estimated_actual_spend: payload
                .savings_estimated_actual_spend
                .as_ref()
                .and_then(Wire::amount)
                .filter(|amount| *amount >= Decimal::ZERO),
        })
    }
}

mod tests;
