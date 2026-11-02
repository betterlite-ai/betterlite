#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::per_second_pricing_cost;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

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

fn record_with_mode(mode: Option<&str>) -> PriceRecord {
    let mut record = PriceRecord::new();
    record.insert("cost_per_second".to_owned(), number("0.01"));
    if let Some(name) = mode {
        record.insert("mode".to_owned(), Scalar::Text(name.to_owned()));
    }
    record
}

#[test]
fn per_second_pricing_cost_an_unmapped_model_comes_back_absent() {
    let lookup = Lookup { record: None };
    let result = per_second_pricing_cost("ghost", None, Some(Decimal::from(2_000_i32)), &lookup)
        .expect("lookup failure is not an error");
    assert_eq!(result, None);
}

#[rstest]
#[case::absent_mode(None)]
#[case::chat(Some("chat"))]
#[case::completion(Some("completion"))]
#[case::embedding(Some("embedding"))]
#[case::responses(Some("responses"))]
fn per_second_pricing_cost_charges_eligible_modes(#[case] mode: Option<&str>) {
    let lookup = Lookup {
        record: Some(record_with_mode(mode)),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(2_000_i32)), &lookup)
        .expect("eligible mode");
    assert_eq!(result, Some((Decimal::new(2, 2), Decimal::ZERO)));
}

#[test]
fn per_second_pricing_cost_declines_an_ineligible_mode() {
    let lookup = Lookup {
        record: Some(record_with_mode(Some("rerank"))),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(2_000_i32)), &lookup)
        .expect("ineligible mode is not an error");
    assert_eq!(result, None);
}

#[test]
fn per_second_pricing_cost_declines_a_model_with_positive_token_prices() {
    let mut record = record_with_mode(None);
    record.insert("input_cost_per_token".to_owned(), number("0.001"));
    let lookup = Lookup {
        record: Some(record),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(2_000_i32)), &lookup)
        .expect("token pricing is not an error");
    assert_eq!(result, None);
}

#[test]
fn per_second_pricing_cost_declines_an_empty_tiered_pricing_table_even_with_zero_token_prices() {
    let mut record = record_with_mode(None);
    record.insert("input_cost_per_token".to_owned(), number("0"));
    record.insert("output_cost_per_token".to_owned(), number("0"));
    record.insert("tiered_pricing".to_owned(), Scalar::Array(Vec::new()));
    let lookup = Lookup {
        record: Some(record),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(2_000_i32)), &lookup)
        .expect("tiered pricing is not an error");
    assert_eq!(result, None);
}

#[test]
fn per_second_pricing_cost_prefers_cost_per_second_over_input_and_output() {
    let mut record = record_with_mode(None);
    record.insert("input_cost_per_second".to_owned(), number("99"));
    record.insert("output_cost_per_second".to_owned(), number("99"));
    let lookup = Lookup {
        record: Some(record),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(1_000_i32)), &lookup)
        .expect("cost_per_second wins");
    assert_eq!(result, Some((Decimal::new(1, 2), Decimal::ZERO)));
}

#[test]
fn per_second_pricing_cost_falls_back_to_input_cost_per_second_without_inspecting_output() {
    let mut record = record_with_mode(None);
    record.remove("cost_per_second");
    record.insert("input_cost_per_second".to_owned(), number("0.02"));
    record.insert(
        "output_cost_per_second".to_owned(),
        Scalar::Array(Vec::new()),
    );
    let lookup = Lookup {
        record: Some(record),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(1_000_i32)), &lookup)
        .expect("a malformed, unselected rate must not fail the request");
    assert_eq!(result, Some((Decimal::new(2, 2), Decimal::ZERO)));
}

#[test]
fn per_second_pricing_cost_retains_an_explicit_zero_duration() {
    let lookup = Lookup {
        record: Some(record_with_mode(None)),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::ZERO), &lookup)
        .expect("eligible model");
    assert_eq!(result, Some((Decimal::ZERO, Decimal::ZERO)));
}

#[test]
fn per_second_pricing_cost_a_missing_response_time_defaults_to_zero_seconds() {
    let lookup = Lookup {
        record: Some(record_with_mode(None)),
    };
    let result = per_second_pricing_cost("model", None, None, &lookup).expect("eligible model");
    assert_eq!(result, Some((Decimal::ZERO, Decimal::ZERO)));
}

#[test]
fn per_second_pricing_cost_retains_a_signed_duration() {
    let lookup = Lookup {
        record: Some(record_with_mode(None)),
    };
    let result = per_second_pricing_cost("model", None, Some(Decimal::from(-2_000_i32)), &lookup)
        .expect("eligible model");
    assert_eq!(result, Some((Decimal::new(-2, 2), Decimal::ZERO)));
}
