#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::{AtomicCounterMeta, CacheCounterValue};
use crate::settlement_and_rollback::SettlementError;
use crate::settlement_and_rollback::refunds::counter_refunds_from_atomic_response as atomic_response_refunds;
use crate::settlement_and_rollback::tests::support::{int_value, meta, text_value};

fn metas(entries: &[(&str, i64)]) -> Vec<AtomicCounterMeta> {
    entries
        .iter()
        .map(|(counter_key, increment)| meta("{model:gpt-4}:window", counter_key, *increment))
        .collect()
}

#[rstest]
#[case::int_window_start(
    vec![int_value(0), int_value(4), int_value(100)],
    vec![("{model:gpt-4}:requests", "100", 1)]
)]
#[case::text_window_start(
    vec![int_value(0), int_value(4), text_value("300")],
    vec![("{model:gpt-4}:requests", "300", 1)]
)]
#[case::two_counters(
    vec![int_value(0), int_value(4), int_value(100), int_value(5), int_value(200)],
    vec![
        ("{model:gpt-4}:requests", "100", 1),
        ("{model:gpt-4}:tokens", "200", 2),
    ]
)]
fn the_refunds_carry_each_counters_window_start(
    #[case] raw: Vec<CacheCounterValue>,
    #[case] expected: Vec<(&str, &str, i64)>,
) {
    let counters = metas(
        &expected
            .iter()
            .map(|(counter_key, _start, increment)| (*counter_key, *increment))
            .collect::<Vec<_>>(),
    );
    let refunds =
        atomic_response_refunds(&raw, &counters).expect("the reply has every window start");
    let projected: Vec<(String, String, i64)> = refunds
        .iter()
        .map(|refund| {
            (
                refund.counter_key.clone(),
                refund.window_start.clone(),
                refund.increment,
            )
        })
        .collect();
    assert_eq!(
        projected,
        expected
            .iter()
            .map(|(counter_key, start, increment)| {
                ((*counter_key).to_owned(), (*start).to_owned(), *increment)
            })
            .collect::<Vec<_>>()
    );
    let window_keys: Vec<&str> = refunds
        .iter()
        .map(|refund| refund.window_key.as_str())
        .collect();
    assert!(window_keys.iter().all(|key| *key == "{model:gpt-4}:window"));
}

#[test]
fn a_float_window_start_is_truncated_like_a_whole_number() {
    let raw = vec![int_value(0), int_value(4), CacheCounterValue::Float(5.9)];
    let refunds = atomic_response_refunds(&raw, &metas(&[("{model:gpt-4}:requests", 1)]))
        .expect("a float window start has a whole form");
    assert_eq!(
        refunds.first().map(|refund| refund.window_start.as_str()),
        Some("5")
    );
}

#[test]
fn a_reply_short_of_the_first_window_start_names_position_two() {
    let raw = vec![int_value(0), int_value(4)];
    let error = atomic_response_refunds(&raw, &metas(&[("{model:gpt-4}:requests", 1)]))
        .expect_err("the reply is too short");
    assert!(matches!(
        error,
        SettlementError::RefundPosition {
            position: 2,
            count: 2
        }
    ));
}

#[test]
fn a_reply_short_of_the_second_window_start_names_position_four() {
    let raw = vec![int_value(0), int_value(4), int_value(100), int_value(5)];
    let counters = metas(&[("{model:gpt-4}:requests", 1), ("{model:gpt-4}:tokens", 1)]);
    let error = atomic_response_refunds(&raw, &counters).expect_err("the reply is too short");
    assert!(matches!(
        error,
        SettlementError::RefundPosition {
            position: 4,
            count: 4
        }
    ));
}

#[test]
fn a_non_numeric_window_start_is_an_error() {
    let raw = vec![int_value(0), int_value(4), text_value("soon")];
    let error = atomic_response_refunds(&raw, &metas(&[("{model:gpt-4}:requests", 1)]))
        .expect_err("the window start is not a number");
    assert!(matches!(error, SettlementError::WindowCounter(_)));
}
