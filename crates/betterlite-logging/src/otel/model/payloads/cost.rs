use super::super::utils::coercion::OtelModelValue;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct LLMCost {
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub input: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub output: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cache_read: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cache_creation: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub tool_usage: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub original: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub discount_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub discount_percent: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub margin_fixed_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub margin_percent: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub margin_total_amount: Option<Decimal>,
}

impl LLMCost {
    #[must_use]
    pub fn from_breakdown(values: &BTreeMap<String, OtelModelValue>) -> Self {
        let number = |name: &str| values.get(name).and_then(OtelModelValue::as_decimal);
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
