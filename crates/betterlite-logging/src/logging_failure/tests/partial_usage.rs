use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr as _;

use crate::fields::usage::Usage;
use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::project::usage_json;
use crate::logging_failure::partial_usage::{
    AssembledResponse, record_assembled_response_for_failure, record_partial_usage_for_failure,
};

use super::support::{build, init, object, text};

fn usage() -> Usage {
    usage_from_mapping(&super::support::params(&[
        ("prompt_tokens", Json::Number(Number::from(2_i64))),
        ("completion_tokens", Json::Number(Number::from(3_i64))),
        ("total_tokens", Json::Number(Number::from(5_i64))),
    ]))
    .expect("the usage fixture parses")
}

fn money(value: &str) -> Decimal {
    Decimal::from_str(value).expect("the fixture is a decimal")
}

struct Assembled {
    usage: Option<Usage>,
}

impl AssembledResponse for Assembled {
    fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }
}

#[test]
fn records_usage_and_cost() {
    let mut state = build(init());
    record_partial_usage_for_failure(&mut state, &usage(), money("0.5"));
    assert_eq!(
        state.model_call_details.get("combined_usage_object"),
        Some(&usage_json(&usage()))
    );
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.5").expect("a number")))
    );
}

#[test]
fn overwrites_previous_values() {
    let mut state = build(init());
    state
        .model_call_details
        .insert("combined_usage_object".to_owned(), text("stale"));
    state.model_call_details.insert(
        "response_cost".to_owned(),
        Json::Number(Number::from(9_i64)),
    );
    record_partial_usage_for_failure(&mut state, &usage(), money("0.25"));
    assert_eq!(
        state.model_call_details.get("combined_usage_object"),
        Some(&usage_json(&usage()))
    );
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.25").expect("a number")))
    );
}

#[test]
fn zero_cost_is_a_float() {
    let mut state = build(init());
    record_partial_usage_for_failure(&mut state, &usage(), Decimal::ZERO);
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.0").expect("a number")))
    );
}

#[test]
fn assembled_response_records_its_usage_and_calculated_cost() {
    let mut state = build(init());
    let assembled = Assembled {
        usage: Some(usage()),
    };
    let recorded =
        record_assembled_response_for_failure(&mut state, &assembled, |_| Some(money("0.75")));
    assert_eq!(recorded, Some(money("0.75")));
    assert_eq!(
        state.model_call_details.get("combined_usage_object"),
        Some(&usage_json(&usage()))
    );
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.75").expect("a number")))
    );
}

#[test]
fn assembled_response_without_a_cost_records_zero() {
    let mut state = build(init());
    let assembled = Assembled {
        usage: Some(usage()),
    };
    let recorded = record_assembled_response_for_failure(&mut state, &assembled, |_| None);
    assert_eq!(recorded, Some(Decimal::ZERO));
    assert_eq!(
        state.model_call_details.get("response_cost"),
        Some(&Json::Number(Number::from_str("0.0").expect("a number")))
    );
}

#[test]
fn assembled_response_without_usage_records_nothing() {
    let mut state = build(init());
    state.model_call_details.insert(
        "combined_usage_object".to_owned(),
        object(&[("prompt_tokens", Json::Number(Number::from(1_i64)))]),
    );
    let assembled = Assembled { usage: None };
    let called = AtomicBool::new(false);
    let recorded = record_assembled_response_for_failure(&mut state, &assembled, |_| {
        called.store(true, Ordering::SeqCst);
        Some(money("1"))
    });
    assert_eq!(recorded, None);
    assert!(!called.load(Ordering::SeqCst));
    assert_eq!(
        state.model_call_details.get("combined_usage_object"),
        Some(&object(&[(
            "prompt_tokens",
            Json::Number(Number::from(1_i64))
        )]))
    );
    assert!(!state.model_call_details.contains_key("response_cost"));
}

#[rstest]
#[case::records_usage_and_cost(records_usage_and_cost)]
#[case::overwrites_previous_values(overwrites_previous_values)]
#[case::zero_cost_is_a_float(zero_cost_is_a_float)]
#[case::assembled_response_records_its_usage_and_calculated_cost(
    assembled_response_records_its_usage_and_calculated_cost
)]
#[case::assembled_response_without_a_cost_records_zero(
    assembled_response_without_a_cost_records_zero
)]
#[case::assembled_response_without_usage_records_nothing(
    assembled_response_without_usage_records_nothing
)]
fn logging_failure_behavior_tests_behavior(#[case] behavior: fn()) {
    behavior();
}
