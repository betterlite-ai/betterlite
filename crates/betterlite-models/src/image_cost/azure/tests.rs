#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::Number;
use serde_json::value::RawValue;

use super::{AzureImageCostRequest, AzureImageResponse, cost_calculator};
use crate::image_cost::usage::ImageUsageDetails;
use crate::image_cost::{ImageBilling, ImageUsageView};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

fn record(entries: &[(&str, &str)]) -> PriceRecord {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), number(value)))
        .collect()
}

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

struct FakeLookup(BTreeMap<String, PriceRecord>);

impl PriceLookup for FakeLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.0.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn empty_lookup() -> FakeLookup {
    FakeLookup(BTreeMap::new())
}

fn run(
    request: &AzureImageCostRequest<'_>,
    lookup: &FakeLookup,
) -> Result<Decimal, super::ImageCostError> {
    let database = TimeZoneDatabase::from_env();
    cost_calculator(request, lookup, &database)
}

#[test]
fn a_non_image_response_is_an_error_even_with_valid_pricing() {
    let model_info = record(&[("output_cost_per_image", "0.1")]);
    let result = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: None,
            n_images: Some(1),
            width: None,
            height: None,
            response: AzureImageResponse::Other,
            moment: moment(),
        },
        &empty_lookup(),
    );
    assert!(matches!(
        result,
        Err(super::ImageCostError::NotImageResponse)
    ));
}

#[test]
fn image_usage_with_input_tokens_prefers_the_token_based_cost_over_a_truthy_output_price() {
    let model_info = record(&[
        ("input_cost_per_token", "0"),
        ("output_cost_per_token", "0"),
        ("output_cost_per_image_token", "3"),
        ("output_cost_per_image", "999"),
    ]);
    let usage = ImageUsageView::Image(ImageUsageDetails {
        input_tokens: Some(0),
        output_tokens: Some(5),
        total_tokens: Some(5),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    });
    let billing = ImageBilling {
        image_count: 1,
        size: None,
        quality: None,
        usage: Some(usage),
    };
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: None,
            n_images: Some(1),
            width: None,
            height: None,
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("a well-formed usage request does not error");
    assert_eq!(cost, Decimal::new(15, 0));
}

#[test]
fn a_zero_output_cost_per_image_falls_through_to_the_pixel_gate_instead_of_winning() {
    let model_info = record(&[("output_cost_per_image", "0")]);
    let billing = ImageBilling {
        image_count: 4,
        size: None,
        quality: None,
        usage: None,
    };
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: None,
            n_images: None,
            width: None,
            height: None,
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("a declared zero still resolves without a catalog match");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_nonzero_pixel_price_delegates_to_the_default_calculator_with_the_original_model_info() {
    let model_info = record(&[("input_cost_per_pixel", "0.001")]);
    let billing = ImageBilling {
        image_count: 0,
        size: None,
        quality: None,
        usage: None,
    };
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: Some("10-x-10"),
            n_images: Some(2),
            width: None,
            height: None,
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("the supplied pixel price is enough to avoid any catalog lookup");
    assert_eq!(cost, Decimal::new(2, 1));
}

#[test]
fn a_positive_exact_integer_width_and_height_overrides_the_requested_size() {
    let model_info = record(&[("input_cost_per_pixel", "0.001")]);
    let billing = ImageBilling {
        image_count: 0,
        size: None,
        quality: None,
        usage: None,
    };
    let width = RawValue::from_string("20".to_owned()).expect("valid JSON");
    let height = RawValue::from_string("5".to_owned()).expect("valid JSON");
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: Some("999-x-999"),
            n_images: Some(1),
            width: Some(&width),
            height: Some(&height),
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("an exact-integer width and height override the requested size");
    assert_eq!(cost, Decimal::new(1, 1));
}

#[test]
fn a_boolean_width_does_not_count_as_an_exact_integer_override() {
    let model_info = record(&[("input_cost_per_pixel", "0.001")]);
    let billing = ImageBilling {
        image_count: 0,
        size: None,
        quality: None,
        usage: None,
    };
    let width = RawValue::from_string("true".to_owned()).expect("valid JSON");
    let height = RawValue::from_string("5".to_owned()).expect("valid JSON");
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: Some("4-x-4"),
            n_images: Some(1),
            width: Some(&width),
            height: Some(&height),
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("a bool falls back to the requested size instead of overriding it");
    assert_eq!(cost, Decimal::new(16, 3));
}

#[test]
fn a_width_too_large_for_i64_is_a_dimension_overflow_error() {
    let model_info = record(&[("input_cost_per_pixel", "0.001")]);
    let billing = ImageBilling {
        image_count: 0,
        size: None,
        quality: None,
        usage: None,
    };
    let width = RawValue::from_string("10000000000000000000".to_owned()).expect("valid JSON");
    let height = RawValue::from_string("5".to_owned()).expect("valid JSON");
    let result = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: Some("4-x-4"),
            n_images: Some(1),
            width: Some(&width),
            height: Some(&height),
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    );
    assert!(matches!(
        result,
        Err(super::ImageCostError::DimensionOverflow)
    ));
}

#[test]
fn an_absent_n_falls_back_to_the_responses_own_returned_image_count() {
    let model_info = record(&[("output_cost_per_image", "2")]);
    let billing = ImageBilling {
        image_count: 3,
        size: None,
        quality: None,
        usage: None,
    };
    let cost = run(
        &AzureImageCostRequest {
            model: "dall-e-3",
            model_info: Some(&model_info),
            size: None,
            n_images: None,
            width: None,
            height: None,
            response: AzureImageResponse::Image(&billing),
            moment: moment(),
        },
        &empty_lookup(),
    )
    .expect("the returned image count is used when n is absent");
    assert_eq!(cost, Decimal::new(6, 0));
}
