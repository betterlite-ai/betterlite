#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{
    flat_image_cost, input_cost_per_pixel, prices_tokens, raw_price, resolve_image_model_info,
};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

fn record_with(key: &str, value: Scalar) -> PriceRecord {
    let mut record = PriceRecord::new();
    record.insert(key.to_owned(), value);
    record
}

struct FakeLookup {
    effective: Option<PriceRecord>,
    raw: Option<PriceRecord>,
}

impl PriceLookup for FakeLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.effective.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        self.raw.clone()
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective.clone().map(|record| PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

#[rstest]
#[case::absent(PriceRecord::new(), false)]
#[case::null(record_with("input_cost_per_token", Scalar::Null), false)]
#[case::explicit_zero(record_with("output_cost_per_image_token", number("0")), true)]
#[case::input_image_token(record_with("input_cost_per_image_token", number("0.002")), true)]
fn prices_tokens_matches_any_declared_nonnull_rate(
    #[case] model_info: PriceRecord,
    #[case] expected: bool,
) {
    assert_eq!(prices_tokens(&model_info), expected);
}

#[test]
fn flat_image_cost_is_zero_without_a_table() {
    let cost = flat_image_cost(None, 4).expect("no table is not an error");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn flat_image_cost_is_zero_without_a_declared_price() {
    let model_info = PriceRecord::new();
    let cost = flat_image_cost(Some(&model_info), 3).expect("missing price is not an error");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn flat_image_cost_multiplies_the_declared_price_by_the_returned_count() {
    let model_info = record_with("output_cost_per_image", number("0.5"));
    let cost = flat_image_cost(Some(&model_info), 4).expect("valid price");
    assert_eq!(cost, Decimal::new(2, 0));
}

#[test]
fn flat_image_cost_rejects_overflow() {
    let model_info = record_with(
        "output_cost_per_image",
        Scalar::Number(Number::from(i64::MAX)),
    );
    let result = flat_image_cost(Some(&model_info), i64::MAX);
    result.expect_err("an overflowing multiply must not wrap");
}

#[test]
fn resolve_image_model_info_without_any_table_is_an_error() {
    let lookup = FakeLookup {
        effective: None,
        raw: None,
    };
    let result = resolve_image_model_info("gpt-image-1", None, None, &lookup);
    result.expect_err("no supplied and no public table is unknown");
}

#[test]
fn resolve_image_model_info_without_a_supplied_table_uses_the_public_entry() {
    let shared = record_with("output_cost_per_image", number("0.04"));
    let lookup = FakeLookup {
        effective: Some(shared),
        raw: None,
    };
    let resolved =
        resolve_image_model_info("gpt-image-1", None, None, &lookup).expect("public entry present");
    assert_eq!(
        raw_price(&resolved, "output_cost_per_image").expect("typed"),
        Some(Decimal::new(4, 2))
    );
}

#[test]
fn resolve_image_model_info_without_a_public_entry_uses_the_supplied_table_alone() {
    let supplied = record_with("output_cost_per_image", number("0.09"));
    let lookup = FakeLookup {
        effective: None,
        raw: None,
    };
    let resolved = resolve_image_model_info("gpt-image-1", None, Some(&supplied), &lookup)
        .expect("supplied stands alone");
    assert_eq!(
        raw_price(&resolved, "output_cost_per_image").expect("typed"),
        Some(Decimal::new(9, 2))
    );
}

#[test]
fn resolve_image_model_info_overlays_the_supplied_table_on_the_public_entry() {
    let mut shared = PriceRecord::new();
    shared.insert("output_cost_per_image".to_owned(), number("0.04"));
    shared.insert("input_cost_per_pixel".to_owned(), number("0.01"));
    let supplied = record_with("output_cost_per_image", number("0.09"));
    let lookup = FakeLookup {
        effective: Some(shared),
        raw: None,
    };
    let resolved = resolve_image_model_info("gpt-image-1", None, Some(&supplied), &lookup)
        .expect("merged table");
    assert_eq!(
        raw_price(&resolved, "output_cost_per_image").expect("typed"),
        Some(Decimal::new(9, 2))
    );
    assert_eq!(
        raw_price(&resolved, "input_cost_per_pixel").expect("typed"),
        Some(Decimal::new(1, 2))
    );
}

#[test]
fn input_cost_per_pixel_prefers_the_resolved_tables_own_rate() {
    let resolved = record_with("input_cost_per_pixel", number("0.00001"));
    let lookup = FakeLookup {
        effective: None,
        raw: None,
    };
    let value = input_cost_per_pixel(&resolved, &lookup).expect("valid rate");
    assert_eq!(value, Decimal::new(1, 5));
}

#[test]
fn input_cost_per_pixel_an_explicit_zero_wins_immediately() {
    let resolved = record_with("input_cost_per_pixel", number("0"));
    let lookup = FakeLookup {
        effective: None,
        raw: Some(record_with("input_cost_per_pixel", number("0.5"))),
    };
    let value = input_cost_per_pixel(&resolved, &lookup).expect("valid rate");
    assert_eq!(value, Decimal::ZERO);
}

#[test]
fn input_cost_per_pixel_falls_through_to_the_catalog_entry_by_key() {
    let mut resolved = PriceRecord::new();
    resolved.insert("key".to_owned(), Scalar::Text("azure/dall-e-3".to_owned()));
    let catalog = record_with("input_cost_per_pixel", number("0.0002"));
    let lookup = FakeLookup {
        effective: None,
        raw: Some(catalog),
    };
    let value = input_cost_per_pixel(&resolved, &lookup).expect("catalog rate");
    assert_eq!(value, Decimal::new(2, 4));
}

#[test]
fn input_cost_per_pixel_without_a_key_defaults_to_zero() {
    let resolved = PriceRecord::new();
    let lookup = FakeLookup {
        effective: None,
        raw: Some(record_with("input_cost_per_pixel", number("0.5"))),
    };
    let value = input_cost_per_pixel(&resolved, &lookup).expect("no key to look up");
    assert_eq!(value, Decimal::ZERO);
}

#[test]
fn input_cost_per_pixel_without_a_catalog_entry_defaults_to_zero() {
    let mut resolved = PriceRecord::new();
    resolved.insert("key".to_owned(), Scalar::Text("azure/dall-e-3".to_owned()));
    let lookup = FakeLookup {
        effective: None,
        raw: None,
    };
    let value = input_cost_per_pixel(&resolved, &lookup).expect("no catalog entry");
    assert_eq!(value, Decimal::ZERO);
}

#[test]
fn input_cost_per_pixel_a_malformed_resolved_string_still_falls_through() {
    let mut resolved = record_with(
        "input_cost_per_pixel",
        Scalar::Text("not-a-number".to_owned()),
    );
    resolved.insert("key".to_owned(), Scalar::Text("azure/dall-e-3".to_owned()));
    let catalog = record_with("input_cost_per_pixel", number("0.0003"));
    let lookup = FakeLookup {
        effective: None,
        raw: Some(catalog),
    };
    let value =
        input_cost_per_pixel(&resolved, &lookup).expect("malformed string is absent, not zero");
    assert_eq!(value, Decimal::new(3, 4));
}

#[rstest]
#[case::missing(PriceRecord::new(), None)]
#[case::null(record_with("input_cost_per_image", Scalar::Null), None)]
#[case::numeric(
    record_with("input_cost_per_image", number("0.01")),
    Some(Decimal::new(1, 2))
)]
fn raw_price_reads_a_strictly_typed_field(
    #[case] record: PriceRecord,
    #[case] expected: Option<Decimal>,
) {
    let value = raw_price(&record, "input_cost_per_image").expect("typed field");
    assert_eq!(value, expected);
}

#[rstest]
#[case::bool(Scalar::Bool(true))]
#[case::text(Scalar::Text("0.01".to_owned()))]
#[case::array(Scalar::Array(Vec::new()))]
fn raw_price_rejects_a_non_numeric_declared_field(#[case] value: Scalar) {
    let record = record_with("input_cost_per_image", value);
    let result = raw_price(&record, "input_cost_per_image");
    result.expect_err("a strictly typed field must not be coerced");
}
