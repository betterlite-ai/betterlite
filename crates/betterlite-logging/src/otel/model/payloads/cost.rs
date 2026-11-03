use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LLMCost {
    pub input: Option<Decimal>,
    pub output: Option<Decimal>,
    pub cache_read: Option<Decimal>,
    pub cache_creation: Option<Decimal>,
    pub tool_usage: Option<Decimal>,
    pub original: Option<Decimal>,
    pub discount_amount: Option<Decimal>,
    pub discount_percent: Option<Decimal>,
    pub margin_fixed_amount: Option<Decimal>,
    pub margin_percent: Option<Decimal>,
    pub margin_total_amount: Option<Decimal>,
}

impl LLMCost {
    pub fn from_breakdown(values: &BTreeMap<String, Value>) -> Self {
        let number = |name: &str| {
            values
                .get(name)
                .and_then(|value| Decimal::from_str(&value.to_string()).ok())
        };
        Self {
            input: number("input_cost"),
            output: number("output_cost"),
            cache_read: number("cache_read_cost"),
            cache_creation: number("cache_creation_cost"),
            tool_usage: number("tool_usage_cost"),
            original: number("original_cost"),
            discount_amount: number("discount_amount"),
            discount_percent: number("discount_percent"),
            margin_fixed_amount: number("margin_fixed_amount"),
            margin_percent: number("margin_percent"),
            margin_total_amount: number("margin_total_amount"),
        }
    }
}

#[path = "cost/tests.rs"]
mod tests;
