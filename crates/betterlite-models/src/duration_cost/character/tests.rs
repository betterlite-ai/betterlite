#![cfg(test)]

use rust_decimal::Decimal;
use serde_json::Number;

use super::{
    CharacterCostRequest, CostMetric, generic_cost_per_character, select_cost_metric_for_model,
};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn record_with(key: &str, value: Scalar) -> PriceRecord {
    let mut record = PriceRecord::new();
    record.insert(key.to_owned(), value);
    record
}

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

struct Lookup {
    record: Option<PriceRecord>,
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.record.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

#[test]
fn select_cost_metric_prefers_character_pricing() {
    let mut record = record_with("input_cost_per_character", number("0.001"));
    record.insert("input_cost_per_token".to_owned(), number("0.002"));
    assert_eq!(
        select_cost_metric_for_model("model", &record).expect("has character pricing"),
        CostMetric::Character
    );
}

#[test]
fn select_cost_metric_falls_back_to_token_pricing() {
    let record = record_with("input_cost_per_token", number("0.002"));
    assert_eq!(
        select_cost_metric_for_model("model", &record).expect("has token pricing"),
        CostMetric::Token
    );
}

#[test]
fn select_cost_metric_an_explicit_zero_character_rate_still_selects_character() {
    let mut record = record_with("input_cost_per_character", number("0"));
    record.insert("input_cost_per_token".to_owned(), number("0.002"));
    assert_eq!(
        select_cost_metric_for_model("model", &record).expect("a present zero still selects it"),
        CostMetric::Character
    );
}

#[test]
fn select_cost_metric_fails_with_the_model_key_when_neither_is_present() {
    let record = PriceRecord::new();
    let error = select_cost_metric_for_model("gpt-x", &record).expect_err("neither price is set");
    assert!(error.to_string().contains("gpt-x"));
}

fn base_request(record: PriceRecord) -> (CharacterCostRequest<'static>, Lookup) {
    (
        CharacterCostRequest {
            model: "gpt-x",
            custom_llm_provider: "vertex_ai-chars",
            prompt_characters: Decimal::from(100_i32),
            completion_characters: Decimal::from(50_i32),
            custom_prompt_cost: None,
            custom_completion_cost: None,
        },
        Lookup {
            record: Some(record),
        },
    )
}

#[test]
fn generic_cost_per_character_prices_both_sides_from_the_catalog() {
    let mut record = record_with("input_cost_per_character", number("0.001"));
    record.insert("output_cost_per_character".to_owned(), number("0.002"));
    let (request, lookup) = base_request(record);
    let result = generic_cost_per_character(&request, &lookup).expect("both sides price");
    assert_eq!(result.prompt_cost, Some(Decimal::new(1, 1)));
    assert_eq!(result.completion_cost, Some(Decimal::new(1, 1)));
}

#[test]
fn generic_cost_per_character_a_supplied_custom_rate_bypasses_the_missing_catalog_field() {
    let (mut request, lookup) = base_request(PriceRecord::new());
    request.custom_prompt_cost = Some(Decimal::new(5, 2));
    let result = generic_cost_per_character(&request, &lookup).expect("custom rate supplied");
    assert_eq!(result.prompt_cost, Some(Decimal::new(500, 2)));
    assert_eq!(result.completion_cost, None);
}

#[test]
fn generic_cost_per_character_a_missing_side_warns_and_returns_none_for_that_side_only() {
    let record = record_with("input_cost_per_character", number("0.001"));
    let (request, lookup) = base_request(record);
    let result = generic_cost_per_character(&request, &lookup).expect("lookup succeeds");
    assert_eq!(result.prompt_cost, Some(Decimal::new(1, 1)));
    assert_eq!(result.completion_cost, None);
}

#[test]
fn generic_cost_per_character_a_wrong_type_side_warns_and_returns_none_for_that_side_only() {
    let mut record = record_with("input_cost_per_character", number("0.001"));
    record.insert(
        "output_cost_per_character".to_owned(),
        Scalar::Array(Vec::new()),
    );
    let (request, lookup) = base_request(record);
    let result = generic_cost_per_character(&request, &lookup).expect("lookup succeeds");
    assert_eq!(result.prompt_cost, Some(Decimal::new(1, 1)));
    assert_eq!(result.completion_cost, None);
}

#[test]
fn generic_cost_per_character_an_unknown_model_raises() {
    let (request, _lookup) = base_request(PriceRecord::new());
    let lookup = Lookup { record: None };
    generic_cost_per_character(&request, &lookup).unwrap_err();
}
