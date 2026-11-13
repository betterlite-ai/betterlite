#![cfg(test)]
use rstest::rstest;
use rust_decimal::Decimal;

use super::batch_id_to_charge_once;

#[rstest]
#[case::retrieve("retrieve_batch", Some("batch_1"), Decimal::new(5, 0), Some("batch_1"))]
#[case::async_retrieve(
    "aretrieve_batch",
    Some("batch_1"),
    Decimal::new(5, 0),
    Some("batch_1")
)]
#[case::smallest_positive_cost("retrieve_batch", Some("b"), Decimal::new(1, 20), Some("b"))]
#[case::zero_cost("retrieve_batch", Some("batch_1"), Decimal::ZERO, None)]
#[case::negative_cost("retrieve_batch", Some("batch_1"), Decimal::new(-1, 0), None)]
#[case::other_call_type("acompletion", Some("batch_1"), Decimal::new(5, 0), None)]
#[case::create_batch("create_batch", Some("batch_1"), Decimal::new(5, 0), None)]
#[case::not_a_batch_response("retrieve_batch", None, Decimal::new(5, 0), None)]
#[case::empty_call_type("", Some("batch_1"), Decimal::new(5, 0), None)]
fn batch_ids_are_charged_once_only_for_priced_retrieves(
    #[case] call_type: &str,
    #[case] batch_id: Option<&str>,
    #[case] cost: Decimal,
    #[case] expected: Option<&str>,
) {
    assert_eq!(batch_id_to_charge_once(call_type, batch_id, cost), expected);
}
