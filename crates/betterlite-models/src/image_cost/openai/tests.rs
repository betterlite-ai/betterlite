#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::{OpenAiImageCostRequest, cost_calculator};
use crate::cost_usage::{CompletionDetails, CostUsage};
use crate::image_cost::ImageUsageView;
use crate::image_cost::usage::ImageUsageDetails;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn model(entries: &[(&str, &str)]) -> PriceRecord {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str::<Scalar>(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
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

fn run(request: &OpenAiImageCostRequest<'_>) -> Decimal {
    let database = TimeZoneDatabase::from_env();
    cost_calculator(request, &NullLookup, &database).expect("a well-formed request does not error")
}

#[test]
fn without_any_usage_falls_back_to_flat_pricing() {
    let model_info = model(&[("output_cost_per_image", "\"0.25\"")]);
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 2,
        usage: None,
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(5, 1));
}

#[test]
fn without_any_model_info_the_price_table_stays_absent_and_flat_cost_is_zero() {
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: None,
        model_info: None,
        image_count: 3,
        usage: None,
        moment: moment(),
    });
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn chat_usage_with_a_completion_detail_goes_straight_through_generic_pricing() {
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"2\""),
    ]);
    let usage = ImageUsageView::Chat(CostUsage {
        prompt_tokens: 0,
        completion_tokens: 4,
        total_tokens: 4,
        prompt_details: None,
        completion_details: Some(CompletionDetails::default()),
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    });
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-4o",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 1,
        usage: Some(&usage),
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(8, 0));
}

#[test]
fn image_usage_with_input_tokens_prefers_the_token_based_cost() {
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"0\""),
        ("output_cost_per_image_token", "\"3\""),
        ("output_cost_per_image", "\"999\""),
    ]);
    let usage = ImageUsageView::Image(ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(5),
        total_tokens: Some(5),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    });
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 1,
        usage: Some(&usage),
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(15, 0));
}

#[test]
fn image_usage_without_a_declared_token_rate_falls_back_to_flat_cost() {
    let model_info = model(&[("output_cost_per_image", "\"0.5\"")]);
    let usage = ImageUsageView::Image(ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(5),
        total_tokens: Some(5),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    });
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 2,
        usage: Some(&usage),
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(1, 0));
}

#[test]
fn bare_chat_usage_without_any_detail_is_priced_as_text() {
    let model_info = model(&[
        ("input_cost_per_token", "\"0\""),
        ("output_cost_per_token", "\"4\""),
    ]);
    let usage = ImageUsageView::Chat(CostUsage {
        prompt_tokens: 0,
        completion_tokens: 3,
        total_tokens: 3,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    });
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-4o",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 1,
        usage: Some(&usage),
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(12, 0));
}

#[test]
fn an_image_usage_without_input_tokens_falls_back_to_flat_cost() {
    let model_info = model(&[("output_cost_per_image", "\"0.5\"")]);
    let usage = ImageUsageView::Image(ImageUsageDetails {
        input_tokens: None,
        output_tokens: Some(5),
        total_tokens: Some(5),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    });
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: None,
        model_info: Some(&model_info),
        image_count: 4,
        usage: Some(&usage),
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(2, 0));
}

#[test]
fn a_falsey_provider_defaults_to_openai_for_pricing_resolution() {
    let model_info = model(&[("output_cost_per_image", "\"0.1\"")]);
    let cost = run(&OpenAiImageCostRequest {
        model: "gpt-image-1",
        custom_llm_provider: Some(""),
        model_info: Some(&model_info),
        image_count: 1,
        usage: None,
        moment: moment(),
    });
    assert_eq!(cost, Decimal::new(1, 1));
}
