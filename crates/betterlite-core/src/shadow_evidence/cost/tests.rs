#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{PriceMap, call_cost};
use crate::shadow_evidence::chat_text;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

// Answers only for the model it was built for, so the price map sees the response it is given
struct ModelPrice {
    model: &'static str,
    cost: Decimal,
}

impl PriceMap for ModelPrice {
    fn completion_cost(&self, response: &Json) -> Option<Decimal> {
        match chat_text::field(response, "model") {
            Some(Json::Text(model)) if model == self.model => Some(self.cost),
            Some(
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => None,
        }
    }
}

fn price(cost: Decimal) -> ModelPrice {
    ModelPrice {
        model: "gpt-4o",
        cost,
    }
}

#[rstest]
#[case::stamped_number(
    r#"{"model":"gpt-4o","_hidden_params":{"response_cost":0.00123}}"#,
    Decimal::new(123, 5)
)]
#[case::stamped_integer(
    r#"{"model":"gpt-4o","_hidden_params":{"response_cost":3}}"#,
    Decimal::new(3, 0)
)]
#[case::stamped_exponent(
    r#"{"model":"gpt-4o","_hidden_params":{"response_cost":1e-3}}"#,
    Decimal::new(1, 3)
)]
#[case::stamped_precision_is_kept(
    r#"{"model":"gpt-4o","_hidden_params":{"response_cost":0.1234567890123456789}}"#,
    Decimal::new(1_234_567_890_123_456_789, 19)
)]
#[case::stamp_wins_over_the_price_map(
    r#"{"model":"gpt-4o","usage":{"prompt_tokens":1000},"_hidden_params":{"response_cost":0.5}}"#,
    Decimal::new(5, 1)
)]
fn stamp_is_the_billed_cost(#[case] response: &str, #[case] expected: Decimal) {
    let prices = price(Decimal::new(7, 0));
    assert_eq!(call_cost(&json(response), &prices), expected);
}

#[rstest]
#[case::stamped_text(r#"{"model":"gpt-4o","_hidden_params":{"response_cost":"0.5"}}"#)]
#[case::stamped_null(r#"{"model":"gpt-4o","_hidden_params":{"response_cost":null}}"#)]
#[case::stamped_bool(r#"{"model":"gpt-4o","_hidden_params":{"response_cost":true}}"#)]
#[case::stamped_list(r#"{"model":"gpt-4o","_hidden_params":{"response_cost":[1]}}"#)]
#[case::stamped_unrepresentable(r#"{"model":"gpt-4o","_hidden_params":{"response_cost":1e-40}}"#)]
#[case::hidden_params_not_an_object(r#"{"model":"gpt-4o","_hidden_params":"0.5"}"#)]
#[case::hidden_params_null(r#"{"model":"gpt-4o","_hidden_params":null}"#)]
#[case::hidden_params_absent(r#"{"model":"gpt-4o"}"#)]
fn price_map_answers_without_a_number_stamp(#[case] response: &str) {
    let prices = price(Decimal::new(42, 3));
    assert_eq!(call_cost(&json(response), &prices), Decimal::new(42, 3));
}

#[test]
fn model_without_a_price_bills_zero() {
    let prices = ModelPrice {
        model: "other-model",
        cost: Decimal::new(9, 0),
    };
    assert_eq!(
        call_cost(&json(r#"{"model":"gpt-4o"}"#), &prices),
        Decimal::ZERO
    );
}

#[test]
fn zero_price_stays_zero() {
    let prices = price(Decimal::ZERO);
    assert_eq!(
        call_cost(&json(r#"{"model":"gpt-4o"}"#), &prices),
        Decimal::ZERO
    );
}

#[test]
fn price_map_only_answers_for_its_own_model() {
    let prices = price(Decimal::new(42, 3));
    let unstamped = json(r#"{"model":"gpt-4o"}"#);
    let other_model = json(r#"{"model":"gpt-4o-mini"}"#);
    assert_eq!(call_cost(&unstamped, &prices), Decimal::new(42, 3));
    assert_eq!(call_cost(&other_model, &prices), Decimal::ZERO);
}
