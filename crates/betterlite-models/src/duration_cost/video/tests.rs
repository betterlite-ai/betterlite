#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{VideoCostRequest, default_video_cost_calculator, sanitize_video_resolution};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

struct Lookup {
    entries: Vec<(&'static str, PriceRecord)>,
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, record)| record.clone())
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

#[rstest]
#[case::punctuation_and_mixed_case(" 1,080P ".to_owned(), Some("1080p".to_owned()))]
#[case::unicode_letters("Ultr\u{e4} HD".to_owned(), Some("ultr\u{e4}hd".to_owned()))]
#[case::empty_after_stripping(" ,, ".to_owned(), None)]
#[case::too_long("a".repeat(25), None)]
#[case::exactly_max_length("a".repeat(24), Some("a".repeat(24)))]
fn sanitize_video_resolution_keeps_unicode_alphanumerics_within_length(
    #[case] input: String,
    #[case] expected: Option<String>,
) {
    assert_eq!(sanitize_video_resolution(&input), expected);
}

#[test]
fn default_video_cost_calculator_an_empty_supplied_price_table_bypasses_the_lookup_and_returns_zero()
 {
    let lookup = Lookup {
        entries: Vec::new(),
    };
    let supplied = PriceRecord::new();
    let request = VideoCostRequest {
        model: "veo-3",
        duration_seconds: Decimal::from(4_i32),
        custom_llm_provider: None,
        model_info: Some(&supplied),
        video_resolution: None,
    };
    let cost = default_video_cost_calculator(&request, &lookup)
        .expect("supplied model_info bypasses the raw catalog search");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn default_video_cost_calculator_a_free_video_rate_wins_over_a_resolution_specific_rate() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_video_per_second".to_owned(), number("0"));
    record.insert("output_cost_per_second_1080p".to_owned(), number("5"));
    let lookup = Lookup {
        entries: vec![("veo-3", record)],
    };
    let request = VideoCostRequest {
        model: "veo-3",
        duration_seconds: Decimal::from(4_i32),
        custom_llm_provider: None,
        model_info: None,
        video_resolution: Some("1080p"),
    };
    let cost = default_video_cost_calculator(&request, &lookup).expect("known model");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn default_video_cost_calculator_searches_candidate_keys_in_source_order() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_second".to_owned(), number("2"));
    let lookup = Lookup {
        entries: vec![("veo-3", record)],
    };
    let request = VideoCostRequest {
        model: "vertex_ai/veo-3",
        duration_seconds: Decimal::from(3_i32),
        custom_llm_provider: Some("vertex_ai"),
        model_info: None,
        video_resolution: None,
    };
    let cost = default_video_cost_calculator(&request, &lookup)
        .expect("the provider-stripped candidate matches");
    assert_eq!(cost, Decimal::from(6_i32));
}

#[test]
fn default_video_cost_calculator_an_unknown_model_raises() {
    let lookup = Lookup {
        entries: Vec::new(),
    };
    let request = VideoCostRequest {
        model: "ghost",
        duration_seconds: Decimal::from(1_i32),
        custom_llm_provider: None,
        model_info: None,
        video_resolution: None,
    };
    default_video_cost_calculator(&request, &lookup).unwrap_err();
}

#[test]
fn default_video_cost_calculator_a_known_unpriced_model_returns_zero() {
    let lookup = Lookup {
        entries: vec![("veo-3", PriceRecord::new())],
    };
    let request = VideoCostRequest {
        model: "veo-3",
        duration_seconds: Decimal::from(4_i32),
        custom_llm_provider: None,
        model_info: None,
        video_resolution: None,
    };
    let cost = default_video_cost_calculator(&request, &lookup).expect("known but unpriced model");
    assert_eq!(cost, Decimal::ZERO);
}
