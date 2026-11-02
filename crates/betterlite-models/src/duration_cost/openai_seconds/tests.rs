#![cfg(test)]

use rust_decimal::Decimal;
use serde_json::Number;

use super::{cost_per_second, video_generation_cost};
use crate::duration_cost::video::VideoCostRequest;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

// Gates the record on the expected provider string, proving dispatch by success or failure.
struct Lookup {
    record: Option<PriceRecord>,
    expected_provider: Option<&'static str>,
}

impl Lookup {
    fn with(record: Option<PriceRecord>) -> Self {
        Self {
            record,
            expected_provider: None,
        }
    }

    fn expecting(record: Option<PriceRecord>, expected_provider: &'static str) -> Self {
        Self {
            record,
            expected_provider: Some(expected_provider),
        }
    }
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        _model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        if let Some(expected) = self.expected_provider
            && custom_llm_provider != Some(expected)
        {
            return None;
        }
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
fn cost_per_second_an_absent_provider_defaults_to_openai() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_second".to_owned(), number("0.1"));
    let lookup = Lookup::expecting(Some(record), "openai");
    cost_per_second("whisper-1", None, Decimal::from(2_i32), &lookup).expect("known model");
}

#[test]
fn cost_per_second_an_explicit_empty_provider_is_used_as_given() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_second".to_owned(), number("0.1"));
    let lookup = Lookup::expecting(Some(record), "");
    cost_per_second("whisper-1", Some(""), Decimal::from(2_i32), &lookup).expect("known model");
}

#[test]
fn cost_per_second_a_strictly_positive_output_rate_charges_the_completion_side() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_second".to_owned(), number("0.1"));
    record.insert("input_cost_per_second".to_owned(), number("99"));
    let lookup = Lookup::with(Some(record));
    let (prompt, completion) =
        cost_per_second("tts-1", None, Decimal::from(3_i32), &lookup).expect("known model");
    assert_eq!(prompt, Decimal::ZERO);
    assert_eq!(completion, Decimal::new(3, 1));
}

#[test]
fn cost_per_second_a_non_positive_output_rate_falls_through_to_a_present_non_null_input_rate() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_second".to_owned(), number("0"));
    record.insert("input_cost_per_second".to_owned(), number("-0.5"));
    let lookup = Lookup::with(Some(record));
    let (prompt, completion) =
        cost_per_second("whisper-1", None, Decimal::from(2_i32), &lookup).expect("known model");
    assert_eq!(prompt, Decimal::new(-10, 1));
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn cost_per_second_neither_rate_present_charges_nothing() {
    let lookup = Lookup::with(Some(PriceRecord::new()));
    let (prompt, completion) =
        cost_per_second("whisper-1", None, Decimal::from(2_i32), &lookup).expect("known model");
    assert_eq!(prompt, Decimal::ZERO);
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn cost_per_second_an_unknown_model_raises() {
    let lookup = Lookup::with(None);
    cost_per_second("ghost", None, Decimal::ZERO, &lookup).unwrap_err();
}

#[test]
fn video_generation_cost_an_empty_supplied_price_table_bypasses_the_lookup_and_returns_zero() {
    let lookup = Lookup::with(None);
    let supplied = PriceRecord::new();
    let request = VideoCostRequest {
        model: "sora-2",
        duration_seconds: Decimal::from(4_i32),
        custom_llm_provider: None,
        model_info: Some(&supplied),
        video_resolution: None,
    };
    let cost =
        video_generation_cost(&request, &lookup).expect("supplied model_info bypasses the lookup");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn video_generation_cost_prefers_the_per_video_second_rate_even_when_zero() {
    let mut record = PriceRecord::new();
    record.insert("output_cost_per_video_per_second".to_owned(), number("0"));
    record.insert("output_cost_per_second".to_owned(), number("5"));
    let lookup = Lookup::with(Some(record));
    let request = VideoCostRequest {
        model: "sora-2",
        duration_seconds: Decimal::from(4_i32),
        custom_llm_provider: None,
        model_info: None,
        video_resolution: None,
    };
    let cost = video_generation_cost(&request, &lookup).expect("known model");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn video_generation_cost_an_unknown_model_raises() {
    let lookup = Lookup::with(None);
    let request = VideoCostRequest {
        model: "ghost",
        duration_seconds: Decimal::from(1_i32),
        custom_llm_provider: None,
        model_info: None,
        video_resolution: None,
    };
    video_generation_cost(&request, &lookup).unwrap_err();
}
