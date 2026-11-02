#![cfg(test)]

use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::value::RawValue;

use super::{DetailMap, ImageUsageCostRequest, ImageUsageDetails, image_usage_cost};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn detail_map(entries: &[(&str, &str)]) -> DetailMap {
    entries
        .iter()
        .map(|(key, value)| {
            (
                Box::<str>::from(*key),
                RawValue::from_string((*value).to_owned())
                    .expect("a fixture literal is valid JSON"),
            )
        })
        .collect()
}

fn model(entries: &[(&str, &str)]) -> PriceRecord {
    let mut record = PriceRecord::new();
    for (key, value) in entries {
        let scalar = serde_json::from_str::<Scalar>(value).expect("a fixture scalar parses");
        record.insert((*key).to_owned(), scalar);
    }
    record
}

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
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

struct FixedLookup(PricedModel);

impl PriceLookup for FixedLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        Some(self.0.record.clone())
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        Some(self.0.clone())
    }
}

fn run(
    usage: &ImageUsageDetails,
    model_info: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Option<Decimal> {
    let database = TimeZoneDatabase::from_env();
    image_usage_cost(
        &ImageUsageCostRequest {
            model: "gpt-image-1",
            custom_llm_provider: None,
            usage,
            model_info,
            moment: moment(),
        },
        lookup,
        &database,
    )
    .expect("a well-formed usage request does not error")
}

#[test]
fn missing_any_of_the_three_counts_returns_none() {
    let usage = ImageUsageDetails {
        input_tokens: Some(5),
        output_tokens: None,
        total_tokens: Some(5),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    assert_eq!(run(&usage, None, &NullLookup), None);
}

#[test]
fn an_all_zero_triple_is_missing_metadata() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(0),
        total_tokens: Some(0),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    assert_eq!(run(&usage, None, &NullLookup), None);
}

#[test]
fn a_supplied_table_without_a_declared_token_rate_returns_none() {
    let usage = ImageUsageDetails {
        input_tokens: Some(1),
        output_tokens: Some(1),
        total_tokens: Some(2),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    let model_info = model(&[("output_cost_per_image", "\"0.04\"")]);
    assert_eq!(run(&usage, Some(&model_info), &NullLookup), None);
}

#[test]
fn no_output_detail_at_all_sends_every_output_token_to_image_pricing() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(10),
        total_tokens: Some(10),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
        ("output_cost_per_image_token", "\"9\""),
    ]);
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(cost, Decimal::new(90, 0));
}

#[test]
fn an_empty_completion_detail_object_sends_the_remainder_to_text_pricing() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(10),
        total_tokens: Some(10),
        input_tokens_details: None,
        completion_tokens_details: Some(detail_map(&[])),
        output_tokens_details: None,
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
        ("output_cost_per_image_token", "\"9\""),
    ]);
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(cost, Decimal::new(20, 0));
}

#[test]
fn completion_tokens_details_is_preferred_over_output_tokens_details_even_when_empty() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(10),
        total_tokens: Some(10),
        input_tokens_details: None,
        completion_tokens_details: Some(detail_map(&[])),
        output_tokens_details: Some(detail_map(&[("image_tokens", "10")])),
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
        ("output_cost_per_image_token", "\"9\""),
    ]);
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(
        cost,
        Decimal::new(20, 0),
        "the empty completion record wins over output_tokens_details"
    );
}

#[test]
fn a_populated_output_detail_record_keeps_its_counts_and_assigns_the_remainder_to_text() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(10),
        total_tokens: Some(10),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: Some(detail_map(&[("image_tokens", "4"), ("audio_tokens", "1")])),
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
        ("output_cost_per_image_token", "\"9\""),
        ("output_cost_per_audio_token", "\"3\""),
    ]);
    // known = image(4) + audio(1) = 5, remainder = 10 - 5 = 5 goes to text.
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(cost, Decimal::new(5_i64 * 2 + 4 * 9 + 3, 0));
}

#[test]
fn a_non_integer_shaped_detail_value_counts_as_zero_not_an_error() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(10),
        total_tokens: Some(10),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: Some(detail_map(&[
            ("image_tokens", "4.5"),
            ("audio_tokens", "\"nope\""),
        ])),
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
    ]);
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(
        cost,
        Decimal::new(20, 0),
        "a float or a string detail value counts as zero, not an integer, so the full ten tokens price as text"
    );
}

#[test]
fn input_token_detail_assigns_the_image_share_at_the_image_rate() {
    let usage = ImageUsageDetails {
        input_tokens: Some(5),
        output_tokens: Some(0),
        total_tokens: Some(5),
        input_tokens_details: Some(detail_map(&[("image_tokens", "3")])),
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("input_cost_per_image_token", "\"7\""),
        ("output_cost_per_token", "\"0\""),
    ]);
    let cost = run(&usage, Some(&model_info), &NullLookup).expect("priced");
    assert_eq!(cost, Decimal::new(21, 0));
}

#[test]
fn without_a_supplied_table_the_lookups_priced_model_is_used() {
    let usage = ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(4),
        total_tokens: Some(4),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    };
    let lookup = FixedLookup(PricedModel {
        record: model(&[
            ("input_cost_per_token", "\"0\""),
            ("output_cost_per_token", "\"0\""),
            ("output_cost_per_image_token", "\"1.5\""),
        ]),
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    });
    let cost = run(&usage, None, &lookup).expect("resolved via the catalog lookup");
    assert_eq!(cost, Decimal::new(6, 0));
}
