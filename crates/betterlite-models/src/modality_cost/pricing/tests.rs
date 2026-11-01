#![cfg(test)]

use rust_decimal::Decimal;

use super::{key_has_price, price_for_key, service_tier_cost_key};
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

fn model(entries: &[(&str, Scalar)]) -> ModelInfo {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn no_service_tier_leaves_the_base_key() {
    assert_eq!(
        service_tier_cost_key("input_cost_per_token", None),
        "input_cost_per_token"
    );
}

#[test]
fn a_recognized_service_tier_appends_its_suffix() {
    assert_eq!(
        service_tier_cost_key("input_cost_per_token", Some("flex")),
        "input_cost_per_token_flex"
    );
}

#[test]
fn fast_and_priority_share_the_priority_suffix() {
    assert_eq!(
        service_tier_cost_key("input_cost_per_token", Some("fast")),
        "input_cost_per_token_priority"
    );
}

#[test]
fn an_unrecognized_service_tier_leaves_the_base_key() {
    assert_eq!(
        service_tier_cost_key("input_cost_per_token", Some("auto")),
        "input_cost_per_token"
    );
}

#[test]
fn a_present_suffixed_key_is_used_directly() {
    let model = model(&[(
        "input_cost_per_token_flex",
        Scalar::Number(serde_json::Number::from(2_i64)),
    )]);
    let price = price_for_key(&model, "input_cost_per_token_flex").expect("parsed");
    assert_eq!(price, Some(Decimal::from(2_i64)));
}

#[test]
fn a_missing_suffixed_key_falls_back_to_the_base_key() {
    let model = model(&[(
        "input_cost_per_token",
        Scalar::Number(serde_json::Number::from(3_i64)),
    )]);
    let price = price_for_key(&model, "input_cost_per_token_flex").expect("fell back");
    assert_eq!(price, Some(Decimal::from(3_i64)));
}

#[test]
fn an_explicit_null_suffixed_key_falls_back_like_a_missing_one() {
    let model = model(&[
        ("input_cost_per_token_flex", Scalar::Null),
        (
            "input_cost_per_token",
            Scalar::Number(serde_json::Number::from(3_i64)),
        ),
    ]);
    let price = price_for_key(&model, "input_cost_per_token_flex").expect("fell back");
    assert_eq!(price, Some(Decimal::from(3_i64)));
}

#[test]
fn an_explicit_null_base_key_with_no_fallback_is_none() {
    let model = model(&[("input_cost_per_token_flex", Scalar::Null)]);
    let price = price_for_key(&model, "input_cost_per_token_flex").expect("no fallback");
    assert_eq!(price, None);
}

#[test]
fn key_has_price_is_false_for_null_and_missing() {
    let model = model(&[("a", Scalar::Null)]);
    assert!(!key_has_price(&model, "a"));
    assert!(!key_has_price(&model, "b"));
}

#[test]
fn key_has_price_is_true_for_an_explicit_zero() {
    let model = model(&[("a", Scalar::Number(serde_json::Number::from(0_i64)))]);
    assert!(key_has_price(&model, "a"));
}
