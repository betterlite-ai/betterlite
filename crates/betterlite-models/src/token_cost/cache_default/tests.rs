#![cfg(test)]

use rust_decimal::Decimal;
use serde_json::Number;

use super::{provider_cache_read_default, with_default_cache_read_rate};
use crate::off_peak::schema::{OffPeakBlock, RateField};
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

fn empty_off_peak() -> OffPeakBlock {
    OffPeakBlock {
        hours_utc: None,
        windows: None,
        weekday_timezone: None,
        input_cost_per_token: None,
        output_cost_per_token: None,
        reasoning_token_cost: None,
        cache_read_input_token_cost: None,
        cache_creation_cost: None,
    }
}

fn model_with_input_rate(rate: &str) -> ModelInfo {
    let mut model = ModelInfo::new();
    model.insert(
        "input_cost_per_token".to_owned(),
        Scalar::Number(
            Number::from_f64(rate.parse().expect("test literal is numeric")).expect("finite"),
        ),
    );
    model
}

#[test]
fn non_fireworks_provider_is_left_untouched() {
    let model = model_with_input_rate("0.002");
    let (updated, off_peak) =
        provider_cache_read_default(&model, Some("openai"), None, false).expect("no overflow");
    assert!(!updated.contains_key("cache_read_input_token_cost"));
    assert!(off_peak.is_none());
}

#[test]
fn fireworks_without_a_provider_cache_rate_inserts_half_the_input_rate() {
    let model = model_with_input_rate("0.002");
    let (updated, _) = provider_cache_read_default(&model, Some("fireworks_ai"), None, false)
        .expect("no overflow");
    let Some(Scalar::Text(rate)) = updated.get("cache_read_input_token_cost") else {
        panic!("expected a derived cache read rate");
    };
    assert_eq!(
        rate.parse::<Decimal>().expect("decimal"),
        Decimal::new(1, 3)
    );
}

#[test]
fn an_explicit_nonnull_cache_rate_is_preserved() {
    let mut model = model_with_input_rate("0.002");
    model.insert(
        "cache_read_input_token_cost".to_owned(),
        Scalar::Text("0.0009".to_owned()),
    );
    let (updated, _) = with_default_cache_read_rate(&model, None, false).expect("no overflow");
    let Some(Scalar::Text(rate)) = updated.get("cache_read_input_token_cost") else {
        panic!("expected the original cache read rate");
    };
    assert_eq!(rate, "0.0009");
}

#[test]
fn a_present_null_cache_rate_still_receives_the_default() {
    let mut model = model_with_input_rate("0.002");
    model.insert("cache_read_input_token_cost".to_owned(), Scalar::Null);
    let (updated, _) = with_default_cache_read_rate(&model, None, false).expect("no overflow");
    assert!(matches!(
        updated.get("cache_read_input_token_cost"),
        Some(Scalar::Text(_))
    ));
}

#[test]
fn a_boolean_input_rate_is_excluded_from_the_default() {
    let mut model = ModelInfo::new();
    model.insert("input_cost_per_token".to_owned(), Scalar::Bool(true));
    let (updated, _) = with_default_cache_read_rate(&model, None, false).expect("no overflow");
    assert!(!updated.contains_key("cache_read_input_token_cost"));
}

#[test]
fn the_off_peak_block_gets_its_own_derived_rate_when_its_key_is_absent() {
    let model = model_with_input_rate("0.002");
    let off_peak = OffPeakBlock {
        input_cost_per_token: Some(RateField::Number(Number::from_f64(0.01).expect("finite"))),
        ..empty_off_peak()
    };
    let (_, updated_off_peak) =
        with_default_cache_read_rate(&model, Some(&off_peak), false).expect("no overflow");
    let Some(RateField::Text(rate)) = updated_off_peak
        .expect("off-peak passed through")
        .cache_read_input_token_cost
    else {
        panic!("expected a derived off-peak cache read rate");
    };
    assert_eq!(
        rate.parse::<Decimal>().expect("decimal"),
        Decimal::new(5, 3)
    );
}

#[test]
fn a_present_off_peak_cache_key_blocks_the_off_peak_default() {
    let model = model_with_input_rate("0.002");
    let off_peak = OffPeakBlock {
        cache_read_input_token_cost: None,
        ..empty_off_peak()
    };
    let (_, updated_off_peak) =
        with_default_cache_read_rate(&model, Some(&off_peak), true).expect("no overflow");
    assert!(
        updated_off_peak
            .expect("off-peak passed through")
            .cache_read_input_token_cost
            .is_none(),
        "the present-null key must block insertion even though the field reads as absent"
    );
}

#[test]
fn the_off_peak_block_falls_back_to_the_derived_model_rate_without_its_own_input_rate() {
    let model = model_with_input_rate("0.002");
    let off_peak = OffPeakBlock::default();
    let (_, updated_off_peak) =
        with_default_cache_read_rate(&model, Some(&off_peak), false).expect("no overflow");
    let Some(RateField::Text(rate)) = updated_off_peak
        .expect("off-peak passed through")
        .cache_read_input_token_cost
    else {
        panic!("expected the model's own derived cache read rate");
    };
    assert_eq!(
        rate.parse::<Decimal>().expect("decimal"),
        Decimal::new(1, 3)
    );
}
