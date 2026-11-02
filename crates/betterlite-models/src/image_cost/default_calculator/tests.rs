#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{DefaultImageCostRequest, default_image_cost_calculator, parse_image_dimensions};
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

struct CatalogLookup(BTreeMap<String, PriceRecord>);

impl PriceLookup for CatalogLookup {
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

fn catalog(entries: &[(&str, &str, &str)]) -> CatalogLookup {
    let mut map = BTreeMap::new();
    for (key, price_key, price) in entries {
        map.insert((*key).to_owned(), record_with(price_key, number(price)));
    }
    CatalogLookup(map)
}

#[rstest]
#[case::dash_separator("512-x-512", (512, 512))]
#[case::plain_x("512x512", (512, 512))]
#[case::negative_preserved("-1x5", (-1, 5))]
#[case::zero_preserved("0x0", (0, 0))]
#[case::underscored("1_024x1_024", (1024, 1024))]
fn parse_image_dimensions_accepts_the_grammar(#[case] size: &str, #[case] expected: (i64, i64)) {
    assert_eq!(parse_image_dimensions(size).expect("valid size"), expected);
}

#[rstest]
#[case::three_parts("1x2x3")]
#[case::one_part("1024")]
#[case::non_integer("a-x-b")]
#[case::decimal_point("1.5-x-5")]
fn parse_image_dimensions_rejects_malformed_input(#[case] size: &str) {
    parse_image_dimensions(size).unwrap_err();
}

#[test]
fn the_default_size_is_1024_square_when_size_is_absent() {
    let lookup = catalog(&[("1024-x-1024/dall-e-3", "output_cost_per_image", "0.04")]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("the base candidate matches");
    assert_eq!(cost, Decimal::new(4, 2));
}

#[test]
fn a_quality_candidate_is_tried_before_the_base_candidate() {
    let lookup = catalog(&[
        ("hd/1024-x-1024/dall-e-3", "output_cost_per_image", "0.08"),
        ("1024-x-1024/dall-e-3", "output_cost_per_image", "0.04"),
    ]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: Some("hd"),
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("the quality candidate matches first");
    assert_eq!(cost, Decimal::new(8, 2));
}

#[test]
fn the_provider_first_quality_candidate_is_tried_before_the_base_candidate() {
    let lookup = catalog(&[
        (
            "azure/hd/1024-x-1024/dall-e-3",
            "output_cost_per_image",
            "0.09",
        ),
        (
            "azure/1024-x-1024/dall-e-3",
            "output_cost_per_image",
            "0.04",
        ),
    ]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "azure/dall-e-3",
            custom_llm_provider: Some("azure"),
            quality: Some("hd"),
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("the provider-first quality candidate matches first");
    assert_eq!(cost, Decimal::new(9, 2));
}

#[test]
fn the_fallback_quality_candidate_is_literally_high_not_medium() {
    let lookup = catalog(&[("high/1024-x-1024/dall-e-3", "output_cost_per_image", "0.07")]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("the high/base candidate matches");
    assert_eq!(cost, Decimal::new(7, 2));
}

#[test]
fn provider_prefix_is_stripped_from_every_occurrence_before_matching() {
    let lookup = catalog(&[(
        "azure/1024-x-1024/dall-e-3",
        "output_cost_per_image",
        "0.03",
    )]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "azure/azure/dall-e-3",
            custom_llm_provider: Some("azure"),
            quality: None,
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("every azure/ occurrence is stripped, leaving a single dall-e-3 segment");
    assert_eq!(cost, Decimal::new(3, 2));
}

#[test]
fn a_supplied_table_without_any_catalog_match_still_succeeds() {
    let lookup = catalog(&[]);
    let model_info = record_with("output_cost_per_image", number("0.5"));
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "unlisted-model",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(2),
            size: None,
            model_info: Some(&model_info),
        },
        &lookup,
    )
    .expect("a supplied table stands on its own");
    assert_eq!(cost, Decimal::new(1, 0));
}

#[test]
fn no_catalog_match_and_no_supplied_table_is_an_unknown_model() {
    let lookup = catalog(&[]);
    let result = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "unlisted-model",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    );
    result.unwrap_err();
}

#[test]
fn a_catalog_match_without_any_usable_price_is_a_distinct_failure() {
    let lookup = catalog(&[("1024-x-1024/dall-e-3", "some_other_field", "1")]);
    let result = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(1),
            size: None,
            model_info: None,
        },
        &lookup,
    );
    result.unwrap_err();
}

#[test]
fn the_supplied_table_is_examined_before_the_public_table_and_an_explicit_zero_wins() {
    let lookup = catalog(&[("1024-x-1024/dall-e-3", "output_cost_per_image", "0.9")]);
    let model_info = record_with("output_cost_per_image", number("0"));
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(3),
            size: None,
            model_info: Some(&model_info),
        },
        &lookup,
    )
    .expect("the supplied explicit zero wins over the public price");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_pixel_price_is_scaled_by_height_times_width_times_image_count() {
    let lookup = catalog(&[("2-x-3/dall-e-3", "input_cost_per_pixel", "0.001")]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: Some(2),
            size: Some("2-x-3"),
            model_info: None,
        },
        &lookup,
    )
    .expect("the pixel candidate matches");
    assert_eq!(cost, Decimal::new(12, 3));
}

#[test]
fn n_absent_defaults_to_one_image() {
    let lookup = catalog(&[("1024-x-1024/dall-e-3", "output_cost_per_image", "0.2")]);
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: "dall-e-3",
            custom_llm_provider: None,
            quality: None,
            n_images: None,
            size: None,
            model_info: None,
        },
        &lookup,
    )
    .expect("n defaults to one");
    assert_eq!(cost, Decimal::new(2, 1));
}

#[rstest]
fn replays_the_image_dimensions_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/image_dimensions");
    for entry in fs::read_dir(directory).expect("the image_dimensions corpus directory exists") {
        let path = entry.expect("a corpus entry is readable").path();
        if let Ok(text) = fs::read_to_string(path) {
            let _parsed = parse_image_dimensions(&text);
        }
    }
}
